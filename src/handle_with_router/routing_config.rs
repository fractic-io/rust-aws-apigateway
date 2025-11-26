use std::collections::HashMap;

use async_trait::async_trait;
use aws_lambda_events::{
    apigw::{ApiGatewayProxyRequest, ApiGatewayProxyResponse},
    http::Method,
};
use lambda_runtime::{Error, LambdaEvent};

use crate::{
    errors::InvalidRouteError,
    shared::{request_processing::RequestMetadata, response_building::build_err},
};

// API Gateway routing config.
// --------------------------------------------------

/// Access control for non-owned routes.
pub enum Access {
    /// Any user, including unauthenticated users.
    Guest,
    /// Any authenticated user.
    User,
    /// Only admin users.
    Admin,
    /// All access is denied.
    None,
}

/// Access control for owned routes.
pub enum OwnedAccess {
    /// Any user, including unauthenticated users.
    Guest,
    /// Any authenticated user.
    User,
    /// Only the resource owner.
    Owner,
    /// Only admin users.
    Admin,
    /// Owner or admin users.
    OwnerOrAdmin,
    /// All access is denied.
    None,
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

pub struct RoutingConfig {
    pub function_routes: HashMap<String, Box<dyn FunctionSpec>>,
    pub crud_routes: HashMap<String, Box<dyn CrudSpec>>,
}

// API Gateway routing utils.
// --------------------------------------------------

impl RoutingConfig {
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
                .and_then(|proxy| self.function_routes.get(proxy))
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
            .and_then(|proxy| self.crud_routes.get(proxy))
            .map(|spec| RouteSpecRef::Crud(spec.as_ref()))
    }
}

enum RouteSpecRef<'a> {
    Function(&'a dyn FunctionSpec),
    Crud(&'a dyn CrudSpec),
}

// Access helpers.
// --------------------------------------------------

pub(crate) fn is_allowed_access(metadata: &RequestMetadata, access: &Access) -> bool {
    match access {
        Access::Guest => true,
        Access::User => metadata.is_authenticated,
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
        OwnedAccess::User => metadata.is_authenticated,
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
