use std::collections::HashMap;

use aws_lambda_events::{
    apigw::{ApiGatewayProxyRequest, ApiGatewayProxyResponse},
    http::Method,
};
use core::future::Future;
use lambda_runtime::{Error, LambdaEvent};
use std::pin::Pin;

use crate::{
    errors::{InvalidRouteError, UnauthorizedError},
    shared::{
        request_processing::{parse_request_metadata, RequestMetadata},
        response_building::build_err,
    },
};

// API Gateway routing config.
// --------------------------------------------------

pub enum AccessLevel {
    Guest,
    User,
    Admin,
    None,
}

type RouteHandler = Box<
    dyn Fn(
        LambdaEvent<ApiGatewayProxyRequest>,
        RequestMetadata,
    ) -> Pin<Box<dyn Future<Output = Result<ApiGatewayProxyResponse, Error>>>>,
>;

pub struct FunctionRoute {
    pub access_level: AccessLevel,
    pub handler: RouteHandler,
}

pub struct CrudRoute {
    pub create_access_level: AccessLevel,
    pub read_access_level: AccessLevel,
    pub update_access_level: AccessLevel,
    pub delete_access_level: AccessLevel,
    pub handler: RouteHandler,
}

pub struct RoutingConfig {
    pub function_routes: HashMap<String, FunctionRoute>,
    pub crud_routes: HashMap<String, CrudRoute>,
}

// API Gateway routing utils.
// --------------------------------------------------

pub fn box_route_handler<T>(
    f: fn(LambdaEvent<ApiGatewayProxyRequest>, RequestMetadata) -> T,
) -> RouteHandler
where
    T: Future<Output = Result<ApiGatewayProxyResponse, Error>> + 'static,
{
    Box::new(move |e, m| Box::pin(f(e, m)))
}

impl RoutingConfig {
    pub async fn handle(
        &self,
        event: LambdaEvent<ApiGatewayProxyRequest>,
    ) -> Result<ApiGatewayProxyResponse, Error> {
        let metadata = match parse_request_metadata(&event.payload) {
            Ok(m) => m,
            Err(e) => return build_err(e),
        };

        let route_search = self
            .find_function_spec(&event)
            .or_else(|| self.find_crud_spec(&event));
        let (handler, access_level) = match route_search {
            Some((handler, access_level)) => (handler, access_level),
            None => return build_err(InvalidRouteError::new(event.payload.path)),
        };

        let is_authenticated_for_route = match access_level {
            AccessLevel::Guest => true,
            AccessLevel::User => metadata.is_authenticated,
            AccessLevel::Admin => metadata.is_authenticated && metadata.is_admin,
            AccessLevel::None => false,
        };

        if is_authenticated_for_route {
            handler(event, metadata).await
        } else {
            build_err(UnauthorizedError::new())
        }
    }

    fn find_function_spec<'a>(
        &'a self,
        event: &LambdaEvent<ApiGatewayProxyRequest>,
    ) -> Option<(&'a RouteHandler, &'a AccessLevel)> {
        let method = &event.payload.http_method;
        if method == Method::POST {
            event
                .payload
                .path_parameters
                .get("proxy")
                .and_then(|proxy| self.function_routes.get(proxy))
                .map(|route| (&route.handler, &route.access_level))
        } else {
            None
        }
    }

    fn find_crud_spec<'a>(
        &'a self,
        event: &LambdaEvent<ApiGatewayProxyRequest>,
    ) -> Option<(&'a RouteHandler, &'a AccessLevel)> {
        let method = &event.payload.http_method;
        event
            .payload
            .path_parameters
            .get("proxy")
            .and_then(|proxy| self.crud_routes.get(proxy))
            .map(|route| {
                (
                    &route.handler,
                    match method {
                        &Method::POST => &route.create_access_level,
                        &Method::GET => &route.read_access_level,
                        &Method::PUT => &route.update_access_level,
                        &Method::DELETE => &route.delete_access_level,
                        _ => &AccessLevel::None,
                    },
                )
            })
    }
}
