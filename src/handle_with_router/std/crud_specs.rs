use async_trait::async_trait;
use aws_lambda_events::{
    apigw::{ApiGatewayProxyRequest, ApiGatewayProxyResponse},
    http::Method,
};
use fractic_aws_dynamo::schema::{DynamoObject, PkSk};
use fractic_server_error::{CriticalError, ServerError};
use lambda_runtime::Error;
use serde::de::DeserializeOwned;
use std::pin::Pin;

use crate::{
    errors::{InvalidRequestError, UnauthorizedError},
    handle_with_router::routing_config::{is_allowed_access, is_allowed_owned_access, CrudSpec},
    shared::{
        request_processing::{parse_request_data, parse_request_metadata},
        response_building::{build_err, build_result},
    },
    CrudAccess, OwnedCrudAccess, Validation,
};

pub enum CrudOperation<T: DynamoObject> {
    Create {
        parent_id: Option<PkSk>,
        after: Option<PkSk>,
        data: T::Data,
    },
    Read {
        id: PkSk,
    },
    Update {
        item: T,
    },
    Delete {
        id: PkSk,
    },
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
    access: CrudAccess,
    validation: Validation<CrudOperation<T>>,
    handler: BoxedCrudHandler<T, O>,
}

impl<T, O> Crud<T, O>
where
    T: DynamoObject + DeserializeOwned + Send + 'static,
    O: serde::Serialize + Send + 'static,
{
    pub fn new<H, Fut>(
        access: CrudAccess,
        validation: Validation<CrudOperation<T>>,
        handler: H,
    ) -> Box<dyn CrudSpec>
    where
        H: Fn(CrudOperation<T>) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = Result<O, ServerError>> + Send + 'static,
    {
        Box::new(Self {
            access,
            validation,
            handler: Box::new(move |op| Box::pin(handler(op))),
        })
    }
}

#[async_trait]
impl<T, O> CrudSpec for Crud<T, O>
where
    T: DynamoObject + DeserializeOwned + Send + 'static,
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
        let method = &request.http_method;
        let (allowed, op) = match method {
            &Method::POST => {
                let parent_id = match get_optional_pksk(request, "parent_id") {
                    Ok(v) => v,
                    Err(e) => return build_err(e),
                };
                let after = match get_optional_pksk(request, "after") {
                    Ok(v) => v,
                    Err(e) => return build_err(e),
                };
                let data = match parse_request_data::<T::Data>(request) {
                    Ok(d) => d,
                    Err(e) => return build_err(e),
                };
                (
                    is_allowed_access(&metadata, &self.access.create),
                    CrudOperation::Create {
                        parent_id,
                        after,
                        data,
                    },
                )
            }
            &Method::GET => {
                let id = match get_required_id(request) {
                    Ok(id) => id,
                    Err(e) => return build_err(e),
                };
                (
                    is_allowed_access(&metadata, &self.access.read),
                    CrudOperation::Read { id },
                )
            }
            &Method::PUT => {
                let item = match parse_request_data::<T>(request) {
                    Ok(i) => i,
                    Err(e) => return build_err(e),
                };
                (
                    is_allowed_access(&metadata, &self.access.update),
                    CrudOperation::Update { item },
                )
            }
            &Method::DELETE => {
                let id = match get_required_id(request) {
                    Ok(id) => id,
                    Err(e) => return build_err(e),
                };
                (
                    is_allowed_access(&metadata, &self.access.delete),
                    CrudOperation::Delete { id },
                )
            }
            _ => return build_err(CriticalError::new("unsupported HTTP method for CRUD route")),
        };
        if !allowed {
            return build_err(UnauthorizedError::new());
        }
        build_result((self.handler)(op).await)
    }
}

/// Owned CRUD spec with per-operation access controls and ownership extraction.
pub struct OwnedCrud<T, O>
where
    T: DynamoObject + DeserializeOwned + Send + 'static,
    O: serde::Serialize + Send + 'static,
{
    owner_of_id: Box<dyn Fn(&PkSk) -> Option<String> + Send + Sync>,
    owner_of_parent_id: Box<dyn Fn(&PkSk) -> Option<String> + Send + Sync>,
    access: OwnedCrudAccess,
    validation: Validation<CrudOperation<T>>,
    handler: BoxedCrudHandler<T, O>,
}

impl<T, O> OwnedCrud<T, O>
where
    T: DynamoObject + DeserializeOwned + Send + 'static,
    O: serde::Serialize + Send + 'static,
{
    pub fn new<H, Fut, FOwnerId, FOwnerParentId>(
        owner_of_id: FOwnerId,
        owner_of_parent_id: FOwnerParentId,
        access: OwnedCrudAccess,
        validation: Validation<CrudOperation<T>>,
        handler: H,
    ) -> Box<dyn CrudSpec>
    where
        FOwnerId: Fn(&PkSk) -> Option<String> + Send + Sync + 'static,
        FOwnerParentId: Fn(&PkSk) -> Option<String> + Send + Sync + 'static,
        H: Fn(CrudOperation<T>) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = Result<O, ServerError>> + Send + 'static,
    {
        Box::new(Self {
            owner_of_id: Box::new(owner_of_id),
            owner_of_parent_id: Box::new(owner_of_parent_id),
            access,
            validation,
            handler: Box::new(move |op| Box::pin(handler(op))),
        })
    }
}

#[async_trait]
impl<T, O> CrudSpec for OwnedCrud<T, O>
where
    T: DynamoObject + DeserializeOwned + Send + 'static,
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
        let method = &request.http_method;
        let (authorized, op) = match method {
            &Method::POST => {
                let parent_id = match get_optional_pksk(request, "parent_id") {
                    Ok(v) => v,
                    Err(e) => return build_err(e),
                };
                let after = match get_optional_pksk(request, "after") {
                    Ok(v) => v,
                    Err(e) => return build_err(e),
                };
                let data = match parse_request_data::<T::Data>(request) {
                    Ok(d) => d,
                    Err(e) => return build_err(e),
                };
                let owner = match parent_id {
                    Some(ref pid) => (self.owner_of_parent_id)(pid),
                    None => (self.owner_of_parent_id)(&PkSk::root()),
                };
                let authorized =
                    is_allowed_owned_access(&metadata, &self.access.create, owner.as_deref());
                (
                    authorized,
                    CrudOperation::Create {
                        parent_id,
                        after,
                        data,
                    },
                )
            }
            &Method::GET => {
                let id = match get_required_id(request) {
                    Ok(id) => id,
                    Err(e) => return build_err(e),
                };
                let owner = (self.owner_of_id)(&id);
                let authorized =
                    is_allowed_owned_access(&metadata, &self.access.read, owner.as_deref());
                (authorized, CrudOperation::Read { id })
            }
            &Method::PUT => {
                let item = match parse_request_data::<T>(request) {
                    Ok(i) => i,
                    Err(e) => return build_err(e),
                };
                let owner = (self.owner_of_id)(item.id());
                let authorized =
                    is_allowed_owned_access(&metadata, &self.access.update, owner.as_deref());
                (authorized, CrudOperation::Update { item })
            }
            &Method::DELETE => {
                let id = match get_required_id(request) {
                    Ok(id) => id,
                    Err(e) => return build_err(e),
                };
                let owner = (self.owner_of_id)(&id);
                let authorized =
                    is_allowed_owned_access(&metadata, &self.access.delete, owner.as_deref());
                (authorized, CrudOperation::Delete { id })
            }
            _ => return build_err(CriticalError::new("unsupported HTTP method for CRUD route")),
        };
        if !authorized {
            return build_err(UnauthorizedError::new());
        }
        build_result((self.handler)(op).await)
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

fn get_optional_pksk(
    request: &ApiGatewayProxyRequest,
    key: &str,
) -> Result<Option<PkSk>, ServerError> {
    Ok(match request.query_string_parameters.first(key) {
        Some(val) => Some(
            PkSk::from_string(val)
                .map_err(|e| InvalidRequestError::with_debug(&format!("invalid {}", key), &e))?,
        ),
        None => None,
    })
}
