use std::collections::HashMap;

use async_trait::async_trait;
use aws_lambda_events::{
    apigw::{ApiGatewayProxyRequest, ApiGatewayProxyResponse},
    http::Method,
};
use fractic_server_error::{define_sensitive_error, ServerError};
use lambda_runtime::{Error, LambdaEvent};

use crate::{
    errors::InvalidRouteError,
    shared::{request_processing::RequestMetadata, response_building::build_err},
};

define_sensitive_error!(
    RequireAnyWasFalse,
    "At least one validation rule must be true. Errors:\n{errors:?}",
    { errors: Vec<ServerError> }
);

// API Gateway routing config.
// --------------------------------------------------

/// Access control for non-owned routes.
#[derive(Debug, Default)]
pub enum Access {
    /// Any user, including unauthenticated users.
    Guest,
    /// Any authenticated user.
    AnyUser,
    /// Only admin users.
    Admin,
    /// All access is denied.
    #[default]
    None,
}

#[derive(Debug, Default)]
pub struct CrudAccess {
    pub create: Access,
    pub read: Access,
    pub update: Access,
    pub delete: Access,
}

/// Access control for owned routes.
#[derive(Debug, Default)]
pub enum OwnedAccess {
    /// Any user, including unauthenticated users.
    Guest,
    /// Any authenticated user.
    AnyUser,
    /// Only the resource owner.
    Owner,
    /// Only admin users.
    Admin,
    /// Owner or admin users.
    OwnerOrAdmin,
    /// All access is denied.
    #[default]
    None,
}

#[derive(Debug, Default)]
pub struct OwnedCrudAccess {
    pub create: OwnedAccess,
    pub read: OwnedAccess,
    pub update: OwnedAccess,
    pub delete: OwnedAccess,
}

/// Trait implemented by function route specifications.
#[async_trait]
pub trait FunctionSpec: Send + Sync {
    async fn resolve(
        &self,
        request: &ApiGatewayProxyRequest,
    ) -> Result<ApiGatewayProxyResponse, Error>;
}

/// Trait implemented by CRUD route specifications.
#[async_trait]
pub trait CrudSpec: Send + Sync {
    async fn resolve(
        &self,
        request: &ApiGatewayProxyRequest,
    ) -> Result<ApiGatewayProxyResponse, Error>;
}

pub enum Validation<T> {
    None,
    Require(Box<dyn ValidatorSpec<T>>),
    RequireAll(Vec<Box<dyn ValidatorSpec<T>>>),
    RequireAny(Vec<Box<dyn ValidatorSpec<T>>>),
}

pub trait ValidatorSpec<T>: Send + Sync {
    fn validate(
        &self,
        request: &ApiGatewayProxyRequest,
        data: &T,
        metadata: &RequestMetadata,
    ) -> Result<(), ServerError>;
}

pub struct RoutingConfig {
    pub function_routes: HashMap<&'static str, Box<dyn FunctionSpec>>,
    pub crud_routes: HashMap<&'static str, Box<dyn CrudSpec>>,
}

// API Gateway routing utils.
// --------------------------------------------------

impl RoutingConfig {
    // NOTE: Called by macro-generated tokens, so must be publically visible.
    pub async fn handle(
        &self,
        event: LambdaEvent<ApiGatewayProxyRequest>,
    ) -> Result<ApiGatewayProxyResponse, Error> {
        let route_spec = self
            .find_function_spec(&event)
            .or_else(|| self.find_crud_spec(&event));
        match route_spec {
            Some(RouteSpecRef::Function(spec)) => spec.resolve(&event.payload).await,
            Some(RouteSpecRef::Crud(spec)) => spec.resolve(&event.payload).await,
            None => build_err(InvalidRouteError::new(event.payload.path)),
        }
    }

    fn find_function_spec<'a>(
        &'a self,
        event: &LambdaEvent<ApiGatewayProxyRequest>,
    ) -> Option<RouteSpecRef<'a>> {
        let method = &event.payload.http_method;
        if method == Method::POST {
            event
                .payload
                .path_parameters
                .get("proxy")
                .and_then(|proxy| self.function_routes.get(proxy.as_str()))
                .map(|spec| RouteSpecRef::Function(spec.as_ref()))
        } else {
            None
        }
    }

    fn find_crud_spec<'a>(
        &'a self,
        event: &LambdaEvent<ApiGatewayProxyRequest>,
    ) -> Option<RouteSpecRef<'a>> {
        event
            .payload
            .path_parameters
            .get("proxy")
            .and_then(|proxy| self.crud_routes.get(proxy.as_str()))
            .map(|spec| RouteSpecRef::Crud(spec.as_ref()))
    }
}

enum RouteSpecRef<'a> {
    Function(&'a dyn FunctionSpec),
    Crud(&'a dyn CrudSpec),
}

impl<T> Validation<T> {
    pub(crate) fn validate(
        &self,
        request: &ApiGatewayProxyRequest,
        data: &T,
        metadata: &RequestMetadata,
    ) -> Result<(), ServerError> {
        match self {
            Validation::None => Ok(()),
            Validation::Require(v) => v.validate(request, data, metadata),
            Validation::RequireAll(vs) => {
                for v in vs {
                    v.validate(request, data, metadata)?;
                }
                Ok(())
            }
            Validation::RequireAny(vs) => {
                let mut errors = Vec::new();
                for v in vs {
                    match v.validate(request, data, metadata) {
                        Ok(()) => return Ok(()),
                        Err(e) => errors.push(e),
                    }
                }
                Err(RequireAnyWasFalse::new(errors))
            }
        }
    }
}

// Access helpers.
// --------------------------------------------------

pub(crate) fn is_allowed_access(metadata: &RequestMetadata, access: &Access) -> bool {
    match access {
        Access::Guest => true,
        Access::AnyUser => metadata.is_authenticated,
        Access::Admin => metadata.is_authenticated && metadata.is_admin,
        Access::None => false,
    }
}

pub(crate) fn is_allowed_owned_access(
    metadata: &RequestMetadata,
    access: &OwnedAccess,
    owner: Option<&str>,
) -> bool {
    match access {
        OwnedAccess::Guest => true,
        OwnedAccess::AnyUser => metadata.is_authenticated,
        OwnedAccess::Admin => metadata.is_authenticated && metadata.is_admin,
        OwnedAccess::Owner => match (owner, &metadata.user_sub) {
            (Some(owner_sub), Some(user_sub)) => owner_sub == user_sub,
            _ => false,
        },
        OwnedAccess::OwnerOrAdmin => {
            if metadata.is_authenticated && metadata.is_admin {
                true
            } else {
                match (owner, &metadata.user_sub) {
                    (Some(owner_sub), Some(user_sub)) => owner_sub == user_sub,
                    _ => false,
                }
            }
        }
        OwnedAccess::None => false,
    }
}

/// Before further processing, can quickly check if the request can at least be
/// authorized in principle, assuming the owner later matches. Once owner is
/// known, final check should be made with `is_allowed_owned_access`.
pub(crate) fn preliminary_access_check(metadata: &RequestMetadata, access: &OwnedAccess) -> bool {
    match access {
        OwnedAccess::Guest => true,
        OwnedAccess::AnyUser => metadata.is_authenticated,
        OwnedAccess::Admin => metadata.is_authenticated && metadata.is_admin,
        OwnedAccess::Owner => metadata.is_authenticated,
        OwnedAccess::OwnerOrAdmin => {
            if metadata.is_authenticated && metadata.is_admin {
                true
            } else {
                metadata.is_authenticated
            }
        }
        OwnedAccess::None => false,
    }
}
