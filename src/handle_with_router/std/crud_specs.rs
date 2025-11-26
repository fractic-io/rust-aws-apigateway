use std::pin::Pin;

use aws_lambda_events::{
    apigw::{ApiGatewayProxyRequest, ApiGatewayProxyResponse},
    http::Method,
};
use fractic_aws_dynamo::schema::{DynamoObject, PkSk};
use fractic_server_error::{CriticalError, ServerError};
use lambda_runtime::Error;
use serde::de::DeserializeOwned;

use crate::{
    errors::InvalidRequestError,
    handle_with_router::routing_config::{CrudSpec, OwnedAccess},
    shared::{
        request_processing::{parse_request_data, parse_request_metadata},
        response_building::{build_err, build_result},
    },
};

use super::validators::Verifier;

pub enum CrudOperation<T> {
    Create { item: T },
    Read { id: PkSk },
    Update { item: T },
    Delete { id: PkSk },
}

type BoxedCrudHandler<T, O> = Box<
    dyn Fn(
            CrudOperation<T>,
        ) -> Pin<Box<dyn std::future::Future<Output = Result<O, ServerError>> + Send>>
        + Send
        + Sync,
>;

/// Non-owned CRUD spec with per-operation access controls.
pub struct Crud<T, O>
where
    T: DynamoObject + DeserializeOwned + Send + 'static,
    O: serde::Serialize + Send + 'static,
{
    pub create_access: crate::handle_with_router::routing_config::Access,
    pub read_access: crate::handle_with_router::routing_config::Access,
    pub update_access: crate::handle_with_router::routing_config::Access,
    pub delete_access: crate::handle_with_router::routing_config::Access,
    handler: BoxedCrudHandler<T, O>,
    verifiers: Vec<Box<dyn Verifier>>,
}

impl<T, O> Crud<T, O>
where
    T: DynamoObject + DeserializeOwned + Send + 'static,
    O: serde::Serialize + Send + 'static,
{
    pub fn new<H, Fut>(
        create_access: crate::handle_with_router::routing_config::Access,
        read_access: crate::handle_with_router::routing_config::Access,
        update_access: crate::handle_with_router::routing_config::Access,
        delete_access: crate::handle_with_router::routing_config::Access,
        handler: H,
    ) -> Self
    where
        H: Fn(CrudOperation<T>) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = Result<O, ServerError>> + Send + 'static,
    {
        Self {
            create_access,
            read_access,
            update_access,
            delete_access,
            handler: Box::new(move |op| Box::pin(handler(op))),
            verifiers: Vec::new(),
        }
    }

    pub fn with_verifiers(mut self, verifiers: Vec<Box<dyn Verifier>>) -> Self {
        self.verifiers = verifiers;
        self
    }
}

impl<T, O> CrudSpec for Crud<T, O>
where
    T: DynamoObject + DeserializeOwned + Send + 'static,
    O: serde::Serialize + Send + 'static,
{
    fn resolve(
        &self,
        request: &ApiGatewayProxyRequest,
    ) -> Pin<Box<dyn std::future::Future<Output = Result<ApiGatewayProxyResponse, Error>> + Send>>
    {
        let handler = &self.handler;
        let create_access = &self.create_access;
        let read_access = &self.read_access;
        let update_access = &self.update_access;
        let delete_access = &self.delete_access;
        Box::pin(async move {
            let metadata = match parse_request_metadata(request) {
                Ok(m) => m,
                Err(e) => return build_err(e),
            };
            let method = &request.http_method;
            let (allowed, op) = match method {
                &Method::POST => {
                    let item = match parse_request_data::<T>(request) {
                        Ok(i) => i,
                        Err(e) => return build_err(e),
                    };
                    (
                        is_allowed_access(&metadata, &create_access),
                        CrudOperation::Create { item },
                    )
                }
                &Method::GET => {
                    let id = match get_required_id(request) {
                        Ok(id) => id,
                        Err(e) => return build_err(e),
                    };
                    (
                        is_allowed_access(&metadata, &read_access),
                        CrudOperation::Read { id },
                    )
                }
                &Method::PUT => {
                    let item = match parse_request_data::<T>(request) {
                        Ok(i) => i,
                        Err(e) => return build_err(e),
                    };
                    (
                        is_allowed_access(&metadata, &update_access),
                        CrudOperation::Update { item },
                    )
                }
                &Method::DELETE => {
                    let id = match get_required_id(request) {
                        Ok(id) => id,
                        Err(e) => return build_err(e),
                    };
                    (
                        is_allowed_access(&metadata, &delete_access),
                        CrudOperation::Delete { id },
                    )
                }
                _ => {
                    return build_err(CriticalError::new("unsupported HTTP method for CRUD route"))
                }
            };
            if !allowed {
                return build_err(crate::errors::UnauthorizedError::new());
            }
            build_result(handler(op).await)
        })
    }
}

/// Owned CRUD spec with per-operation access controls and ownership extraction.
pub struct OwnedCrud<T, O>
where
    T: DynamoObject + DeserializeOwned + Send + 'static,
    O: serde::Serialize + Send + 'static,
{
    pub create_access: OwnedAccess,
    pub read_access: OwnedAccess,
    pub update_access: OwnedAccess,
    pub delete_access: OwnedAccess,
    owner_of: Box<dyn Fn(&T) -> String + Send + Sync>,
    handler: BoxedCrudHandler<T, O>,
    verifiers: Vec<Box<dyn Verifier>>,
}

impl<T, O> OwnedCrud<T, O>
where
    T: DynamoObject + DeserializeOwned + Send + 'static,
    O: serde::Serialize + Send + 'static,
{
    pub fn new<H, Fut, FOwner>(
        create_access: OwnedAccess,
        read_access: OwnedAccess,
        update_access: OwnedAccess,
        delete_access: OwnedAccess,
        owner_of: FOwner,
        handler: H,
    ) -> Self
    where
        FOwner: Fn(&T) -> String + Send + Sync + 'static,
        H: Fn(CrudOperation<T>) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = Result<O, ServerError>> + Send + 'static,
    {
        Self {
            create_access,
            read_access,
            update_access,
            delete_access,
            owner_of: Box::new(owner_of),
            handler: Box::new(move |op| Box::pin(handler(op))),
            verifiers: Vec::new(),
        }
    }

    pub fn with_verifiers(mut self, verifiers: Vec<Box<dyn Verifier>>) -> Self {
        self.verifiers = verifiers;
        self
    }
}

impl<T, O> CrudSpec for OwnedCrud<T, O>
where
    T: DynamoObject + DeserializeOwned + Send + 'static,
    O: serde::Serialize + Send + 'static,
{
    fn resolve(
        &self,
        request: &ApiGatewayProxyRequest,
    ) -> Pin<Box<dyn std::future::Future<Output = Result<ApiGatewayProxyResponse, Error>> + Send>>
    {
        let handler = &self.handler;
        let create_access = &self.create_access;
        let read_access = &self.read_access;
        let update_access = &self.update_access;
        let delete_access = &self.delete_access;
        let owner_of = &self.owner_of;
        Box::pin(async move {
            let metadata = match parse_request_metadata(request) {
                Ok(m) => m,
                Err(e) => return build_err(e),
            };
            if !metadata.is_authenticated {
                return build_err(crate::errors::UnauthorizedError::new());
            }
            let method = &request.http_method;
            let (authorized, op) = match method {
                &Method::POST => {
                    let item = match parse_request_data::<T>(request) {
                        Ok(i) => i,
                        Err(e) => return build_err(e),
                    };
                    let authorized =
                        is_authorized_owned(&metadata, create_access, Some(&item), owner_of);
                    (authorized, CrudOperation::Create { item })
                }
                &Method::GET => {
                    let id = match get_required_id(request) {
                        Ok(id) => id,
                        Err(e) => return build_err(e),
                    };
                    let authorized =
                        is_authorized_owned::<T>(&metadata, read_access, None, owner_of);
                    (authorized, CrudOperation::Read { id })
                }
                &Method::PUT => {
                    let item = match parse_request_data::<T>(request) {
                        Ok(i) => i,
                        Err(e) => return build_err(e),
                    };
                    let authorized =
                        is_authorized_owned(&metadata, update_access, Some(&item), owner_of);
                    (authorized, CrudOperation::Update { item })
                }
                &Method::DELETE => {
                    let id = match get_required_id(request) {
                        Ok(id) => id,
                        Err(e) => return build_err(e),
                    };
                    let authorized =
                        is_authorized_owned::<T>(&metadata, delete_access, None, owner_of);
                    (authorized, CrudOperation::Delete { id })
                }
                _ => {
                    return build_err(CriticalError::new("unsupported HTTP method for CRUD route"))
                }
            };
            if !authorized {
                return build_err(crate::errors::UnauthorizedError::new());
            }
            build_result(handler(op).await)
        })
    }
}

fn get_required_id(request: &ApiGatewayProxyRequest) -> Result<PkSk, ServerError> {
    request
        .query_string_parameters
        .first("id")
        .ok_or(InvalidRequestError::new("query parameter 'id' is required"))
        .and_then(|s| {
            PkSk::from_string(s).map_err(|e| InvalidRequestError::with_debug("invalid id", &e))
        })
}

fn is_allowed_access(
    metadata: &crate::shared::request_processing::RequestMetadata,
    access: &crate::handle_with_router::routing_config::Access,
) -> bool {
    match access {
        crate::handle_with_router::routing_config::Access::Guest => true,
        crate::handle_with_router::routing_config::Access::User => metadata.is_authenticated,
        crate::handle_with_router::routing_config::Access::Admin => {
            metadata.is_authenticated && metadata.is_admin
        }
        crate::handle_with_router::routing_config::Access::None => false,
    }
}

fn is_authorized_owned<T>(
    metadata: &crate::shared::request_processing::RequestMetadata,
    access: &OwnedAccess,
    item: Option<&T>,
    owner_of: &Box<dyn Fn(&T) -> String + Send + Sync>,
) -> bool {
    match access {
        OwnedAccess::OwnerOrAdmin => {
            if metadata.is_admin {
                true
            } else if let (Some(item_ref), Some(sub)) = (item, &metadata.user_sub) {
                sub == &owner_of(item_ref)
            } else {
                false
            }
        }
        OwnedAccess::Owner => {
            if let (Some(item_ref), Some(sub)) = (item, &metadata.user_sub) {
                sub == &owner_of(item_ref)
            } else {
                false
            }
        }
        OwnedAccess::None => false,
    }
}
