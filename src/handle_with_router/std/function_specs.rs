use std::pin::Pin;

use aws_lambda_events::apigw::{ApiGatewayProxyRequest, ApiGatewayProxyResponse};
use fractic_server_error::ServerError;
use lambda_runtime::Error;
use serde::de::DeserializeOwned;

use crate::{
    errors::UnauthorizedError,
    handle_with_router::routing_config::{Access, FunctionSpec, OwnedAccess},
    shared::{
        request_processing::{parse_request_data, parse_request_metadata},
        response_building::{build_err, build_result},
    },
};

use super::validators::Verifier;

type BoxFuture<'a, T> = Pin<Box<dyn std::future::Future<Output = T> + Send + 'a>>;

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

impl<O> FunctionSpec for VoidFunction<O>
where
    O: serde::Serialize + Send + 'static,
{
    fn resolve(
        &self,
        request: &ApiGatewayProxyRequest,
    ) -> Pin<Box<dyn std::future::Future<Output = Result<ApiGatewayProxyResponse, Error>> + Send>>
    {
        let access = &self.access;
        let handler = &self.handler;
        Box::pin(async move {
            let metadata = match parse_request_metadata(request) {
                Ok(m) => m,
                Err(e) => return build_err(e),
            };
            if !is_allowed_access(&metadata, access) {
                return build_err(UnauthorizedError::new());
            }
            build_result(handler().await)
        })
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

impl<I, O> FunctionSpec for Function<I, O>
where
    I: DeserializeOwned + Send + 'static,
    O: serde::Serialize + Send + 'static,
{
    fn resolve(
        &self,
        request: &ApiGatewayProxyRequest,
    ) -> Pin<Box<dyn std::future::Future<Output = Result<ApiGatewayProxyResponse, Error>> + Send>>
    {
        let handler = &self.handler;
        let access = &self.access;
        Box::pin(async move {
            let metadata = match parse_request_metadata(request) {
                Ok(m) => m,
                Err(e) => return build_err(e),
            };
            if !is_allowed_access(&metadata, access) {
                return build_err(UnauthorizedError::new());
            }
            let input = match parse_request_data::<I>(request) {
                Ok(i) => i,
                Err(e) => return build_err(e),
            };
            build_result(handler(input).await)
        })
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

impl<I, O> FunctionSpec for OwnedFunction<I, O>
where
    I: DeserializeOwned + Send + 'static,
    O: serde::Serialize + Send + 'static,
{
    fn resolve(
        &self,
        request: &ApiGatewayProxyRequest,
    ) -> Pin<Box<dyn std::future::Future<Output = Result<ApiGatewayProxyResponse, Error>> + Send>>
    {
        let handler = &self.handler;
        let owner_of = &self.owner_of;
        let access = &self.access;
        Box::pin(async move {
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
            // Ownership enforcement
            let authorized = match access {
                OwnedAccess::OwnerOrAdmin => {
                    if metadata.is_admin {
                        true
                    } else {
                        match &metadata.user_sub {
                            Some(sub) => *sub == (owner_of)(&input),
                            None => false,
                        }
                    }
                }
                OwnedAccess::Owner => match &metadata.user_sub {
                    Some(sub) => *sub == (owner_of)(&input),
                    None => false,
                },
                OwnedAccess::None => false,
            };
            if !authorized {
                return build_err(UnauthorizedError::new());
            }
            build_result(handler(input).await)
        })
    }
}

pub struct VoidOwnedFunction<O>
where
    O: serde::Serialize + Send + 'static,
{
    access: OwnedAccess,
    handler: BoxedVoidHandler<O>,
    verifiers: Vec<Box<dyn Verifier>>,
}

impl<O> VoidOwnedFunction<O>
where
    O: serde::Serialize + Send + 'static,
{
    pub fn new<H, Fut>(access: OwnedAccess, handler: H) -> Self
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

impl<O> FunctionSpec for VoidOwnedFunction<O>
where
    O: serde::Serialize + Send + 'static,
{
    fn resolve(
        &self,
        request: &ApiGatewayProxyRequest,
    ) -> Pin<Box<dyn std::future::Future<Output = Result<ApiGatewayProxyResponse, Error>> + Send>>
    {
        let access = &self.access;
        let handler = &self.handler;
        Box::pin(async move {
            let metadata = match parse_request_metadata(request) {
                Ok(m) => m,
                Err(e) => return build_err(e),
            };
            if !metadata.is_authenticated {
                return build_err(UnauthorizedError::new());
            }
            let authorized = match access {
                OwnedAccess::OwnerOrAdmin => metadata.is_admin, // cannot verify owner without input; restrict to admin for safety
                OwnedAccess::Owner => false,
                OwnedAccess::None => false,
            };
            if !authorized {
                return build_err(UnauthorizedError::new());
            }
            build_result(handler().await)
        })
    }
}

fn is_allowed_access(
    metadata: &crate::shared::request_processing::RequestMetadata,
    access: &Access,
) -> bool {
    match access {
        Access::Guest => true,
        Access::User => metadata.is_authenticated,
        Access::Admin => metadata.is_authenticated && metadata.is_admin,
        Access::None => false,
    }
}
