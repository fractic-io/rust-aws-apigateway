use async_trait::async_trait;
use aws_lambda_events::apigw::{ApiGatewayProxyRequest, ApiGatewayProxyResponse};
use fractic_server_error::ServerError;
use lambda_runtime::Error;
use serde::de::DeserializeOwned;
use std::pin::Pin;

use crate::{
    errors::UnauthorizedError,
    handle_with_router::routing_config::{
        is_allowed_access, is_allowed_owned_access, preliminary_access_check, Access, FunctionSpec,
        OwnedAccess,
    },
    shared::{
        request_processing::{parse_request_data, parse_request_metadata},
        response_building::{build_err, build_result},
    },
    Validation,
};

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

pub struct NullaryFunction<O>
where
    O: serde::Serialize + Send + 'static,
{
    access: Access,
    validation: Validation<()>,
    handler: BoxedVoidHandler<O>,
}

impl<O> NullaryFunction<O>
where
    O: serde::Serialize + Send + 'static,
{
    pub fn new<H, Fut>(
        access: Access,
        validation: Validation<()>,
        handler: H,
    ) -> Box<dyn FunctionSpec>
    where
        H: Fn() -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = Result<O, ServerError>> + Send + 'static,
    {
        Box::new(Self {
            access,
            validation,
            handler: Box::new(move || Box::pin(handler())),
        })
    }
}

#[async_trait]
impl<O> FunctionSpec for NullaryFunction<O>
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
    validation: Validation<I>,
    handler: BoxedFuncHandler<I, O>,
}

impl<I, O> Function<I, O>
where
    I: DeserializeOwned + Send + 'static,
    O: serde::Serialize + Send + 'static,
{
    pub fn new<H, Fut>(
        access: Access,
        validation: Validation<I>,
        handler: H,
    ) -> Box<dyn FunctionSpec>
    where
        H: Fn(I) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = Result<O, ServerError>> + Send + 'static,
    {
        Box::new(Self {
            access,
            validation,
            handler: Box::new(move |i| Box::pin(handler(i))),
        })
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
    owner_of: Box<dyn Fn(&I) -> &str + Send + Sync>,
    access: OwnedAccess,
    validation: Validation<I>,
    handler: BoxedFuncHandler<I, O>,
}

impl<I, O> OwnedFunction<I, O>
where
    I: DeserializeOwned + Send + 'static,
    O: serde::Serialize + Send + 'static,
{
    pub fn new<H, Fut, FOwner>(
        owner_of: FOwner,
        access: OwnedAccess,
        validation: Validation<I>,
        handler: H,
    ) -> Box<dyn FunctionSpec>
    where
        FOwner: Fn(&I) -> &str + Send + Sync + 'static,
        H: Fn(I) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = Result<O, ServerError>> + Send + 'static,
    {
        Box::new(Self {
            owner_of: Box::new(owner_of),
            access,
            validation,
            handler: Box::new(move |i| Box::pin(handler(i))),
        })
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
        if !preliminary_access_check(&metadata, &self.access) {
            return build_err(UnauthorizedError::new());
        }
        let input = match parse_request_data::<I>(request) {
            Ok(i) => i,
            Err(e) => return build_err(e),
        };
        let owner = (self.owner_of)(&input);
        if !is_allowed_owned_access(&metadata, &self.access, Some(owner)) {
            return build_err(UnauthorizedError::new());
        }
        build_result((self.handler)(input).await)
    }
}
