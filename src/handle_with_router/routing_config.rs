use std::collections::HashMap;

use aws_lambda_events::{
    apigw::{ApiGatewayProxyRequest, ApiGatewayProxyResponse},
    http::Method,
};
use core::future::Future;
use lambda_runtime::{Error, LambdaEvent};
use std::pin::Pin;

use crate::{errors::InvalidRouteError, shared::response_building::build_err};

// API Gateway routing config.
// --------------------------------------------------

/// Access control for non-owned routes.
pub enum Access {
    Guest,
    User,
    Admin,
    None,
}

/// Access control for owned routes.
pub enum OwnedAccess {
    /// Only the owner of the resource can access.
    Owner,
    /// The owner or an administrator can access.
    OwnerOrAdmin,
    /// No access is allowed.
    None,
}

/// Trait implemented by function route specifications.
pub trait FunctionSpec: Send + Sync {
    fn resolve(
        &self,
        request: &ApiGatewayProxyRequest,
    ) -> Pin<Box<dyn Future<Output = Result<ApiGatewayProxyResponse, Error>> + Send>>;
}

/// Trait implemented by CRUD route specifications.
pub trait CrudSpec: Send + Sync {
    fn resolve(
        &self,
        request: &ApiGatewayProxyRequest,
    ) -> Pin<Box<dyn Future<Output = Result<ApiGatewayProxyResponse, Error>> + Send>>;
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
