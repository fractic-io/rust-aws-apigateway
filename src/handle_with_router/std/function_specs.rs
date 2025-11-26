use async_trait::async_trait;
use aws_lambda_events::apigw::{ApiGatewayProxyRequest, ApiGatewayProxyResponse};
use fractic_server_error::ServerError;
use lambda_runtime::Error;
use serde::de::DeserializeOwned;
use std::pin::Pin;

use crate::{
    errors::UnauthorizedError,
    handle_with_router::routing_config::{
        is_allowed_access, is_allowed_owned_access, Access, FunctionSpec, OwnedAccess,
    },
    shared::{
        request_processing::{parse_request_data, parse_request_metadata},
        response_building::{build_err, build_result},
    },
};

use super::validators::Verifier;

type BoxedFuncHandler<I, O> = Box<
    dyn Fn(I) -> Pin<Box<dyn std::future::Future<Output = Result<O, ServerError>> + Send>>
        + Send
        + Sync,
>;

type BoxedVoidHandler<O> = Box<
    dyn Fn() -> Pin<Box<dyn std::future::Future<Output = Result<O, ServerError>> + Send>>
        + Send
        + Sync,
>;

pub struct VoidFunction<O>
where
    O: serde::Serialize + Send + 'static,
{
    access: Access,
    handler: BoxedVoidHandler<O>,
    verifiers: Vec<Box<dyn Verifier>>,
}

impl<O> VoidFunction<O>
where
    O: serde::Serialize + Send + 'static,
{
    pub fn new<H, Fut>(access: Access, handler: H) -> Self
    where
        H: Fn() -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = Result<O, ServerError>> + Send + 'static,
    {
        Self {
            access,
            handler: Box::new(move || Box::pin(handler())),
            verifiers: Vec::new(),
        }
    }

    pub fn with_verifiers(mut self, verifiers: Vec<Box<dyn Verifier>>) -> Self {
        self.verifiers = verifiers;
        self
    }
}

#[async_trait]
impl<O> FunctionSpec for VoidFunction<O>
where
    O: serde::Serialize + Send + 'static,
{
    async fn resolve(
        &self,
        request: &ApiGatewayProxyRequest,
    ) -> Result<ApiGatewayProxyResponse, Error> {
        let metadata = match parse_request_metadata(request) {
            Ok(m) => m,
            Err(e) => return build_err(e),
        };
        if !is_allowed_access(&metadata, &self.access) {
            return build_err(UnauthorizedError::new());
        }
        build_result((self.handler)().await)
    }
}

pub struct Function<I, O>
where
    I: DeserializeOwned + Send + 'static,
    O: serde::Serialize + Send + 'static,
{
    access: Access,
    handler: BoxedFuncHandler<I, O>,
    verifiers: Vec<Box<dyn Verifier>>,
}

impl<I, O> Function<I, O>
where
    I: DeserializeOwned + Send + 'static,
    O: serde::Serialize + Send + 'static,
{
    pub fn new<H, Fut>(access: Access, handler: H) -> Self
    where
        H: Fn(I) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = Result<O, ServerError>> + Send + 'static,
    {
        Self {
            access,
            handler: Box::new(move |i| Box::pin(handler(i))),
            verifiers: Vec::new(),
        }
    }

    pub fn with_verifiers(mut self, verifiers: Vec<Box<dyn Verifier>>) -> Self {
        self.verifiers = verifiers;
        self
    }
}

#[async_trait]
impl<I, O> FunctionSpec for Function<I, O>
where
    I: DeserializeOwned + Send + 'static,
    O: serde::Serialize + Send + 'static,
{
    async fn resolve(
        &self,
        request: &ApiGatewayProxyRequest,
    ) -> Result<ApiGatewayProxyResponse, Error> {
        let metadata = match parse_request_metadata(request) {
            Ok(m) => m,
            Err(e) => return build_err(e),
        };
        if !is_allowed_access(&metadata, &self.access) {
            return build_err(UnauthorizedError::new());
        }
        let input = match parse_request_data::<I>(request) {
            Ok(i) => i,
            Err(e) => return build_err(e),
        };
        build_result((self.handler)(input).await)
    }
}

pub struct OwnedFunction<I, O>
where
    I: DeserializeOwned + Send + 'static,
    O: serde::Serialize + Send + 'static,
{
    access: OwnedAccess,
    owner_of: Box<dyn Fn(&I) -> String + Send + Sync>,
    handler: BoxedFuncHandler<I, O>,
    verifiers: Vec<Box<dyn Verifier>>,
}

impl<I, O> OwnedFunction<I, O>
where
    I: DeserializeOwned + Send + 'static,
    O: serde::Serialize + Send + 'static,
{
    pub fn new<H, Fut, FOwner>(access: OwnedAccess, owner_of: FOwner, handler: H) -> Self
    where
        FOwner: Fn(&I) -> String + Send + Sync + 'static,
        H: Fn(I) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = Result<O, ServerError>> + Send + 'static,
    {
        Self {
            access,
            owner_of: Box::new(owner_of),
            handler: Box::new(move |i| Box::pin(handler(i))),
            verifiers: Vec::new(),
        }
    }

    pub fn with_verifiers(mut self, verifiers: Vec<Box<dyn Verifier>>) -> Self {
        self.verifiers = verifiers;
        self
    }
}

#[async_trait]
impl<I, O> FunctionSpec for OwnedFunction<I, O>
where
    I: DeserializeOwned + Send + 'static,
    O: serde::Serialize + Send + 'static,
{
    async fn resolve(
        &self,
        request: &ApiGatewayProxyRequest,
    ) -> Result<ApiGatewayProxyResponse, Error> {
        let metadata = match parse_request_metadata(request) {
            Ok(m) => m,
            Err(e) => return build_err(e),
        };
        if !metadata.is_authenticated {
            return build_err(UnauthorizedError::new());
        }
        let input = match parse_request_data::<I>(request) {
            Ok(i) => i,
            Err(e) => return build_err(e),
        };
        let owner = (self.owner_of)(&input);
        let authorized = is_allowed_owned_access(&metadata, &self.access, Some(owner.as_str()));
        if !authorized {
            return build_err(UnauthorizedError::new());
        }
        build_result((self.handler)(input).await)
    }
}
