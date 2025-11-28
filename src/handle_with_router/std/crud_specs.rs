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
    handle_with_router::routing_config::{
        is_allowed_access, is_allowed_owned_access, preliminary_access_check, CrudSpec,
    },
    shared::{
        request_processing::{parse_request_data, parse_request_metadata},
        response_building::{build_err, build_result},
    },
    CrudAccess, OwnedCrudAccess, Validation,
};

pub enum CrudOperation<T: DynamoObject> {
    List {
        parent_id: Option<PkSk>,
    },
    Create {
        parent_id: Option<PkSk>,
        after: Option<PkSk>,
        data: T::Data,
    },
    CreateMultiple {
        parent_id: Option<PkSk>,
        after: Option<PkSk>,
        data: Vec<T::Data>,
    },
    Read {
        id: PkSk,
    },
    ReadMultiple {
        ids: Vec<PkSk>,
    },
    Update {
        item: T,
    },
    Delete {
        id: PkSk,
        non_recursive: bool,
    },
    DeleteMultiple {
        ids: Vec<PkSk>,
        non_recursive: bool,
    },
    DeleteAll {
        parent_id: Option<PkSk>,
        non_recursive: bool,
    },
    ReplaceAll {
        parent_id: Option<PkSk>,
        data: Vec<T::Data>,
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
        let op = match method {
            &Method::POST => {
                if has_flag(request, "replace_all") {
                    if !is_allowed_access(&metadata, &self.access.replace_all) {
                        return build_err(UnauthorizedError::new());
                    }
                    let parent_id = match get_optional_pksk(request, "parent_id") {
                        Ok(v) => v,
                        Err(e) => return build_err(e),
                    };
                    let data = match parse_request_data::<Vec<T::Data>>(request) {
                        Ok(d) => d,
                        Err(e) => return build_err(e),
                    };
                    CrudOperation::ReplaceAll { parent_id, data }
                } else {
                    let parent_id = match get_optional_pksk(request, "parent_id") {
                        Ok(v) => v,
                        Err(e) => return build_err(e),
                    };
                    let after = match get_optional_pksk(request, "after") {
                        Ok(v) => v,
                        Err(e) => return build_err(e),
                    };
                    // Batch create if body is a list; fall back to single.
                    match parse_request_data::<Vec<T::Data>>(request) {
                        Ok(list) => {
                            if !is_allowed_access(&metadata, &self.access.batch_create) {
                                return build_err(UnauthorizedError::new());
                            }
                            CrudOperation::CreateMultiple {
                                parent_id,
                                after,
                                data: list,
                            }
                        }
                        Err(_) => {
                            if !is_allowed_access(&metadata, &self.access.create) {
                                return build_err(UnauthorizedError::new());
                            }
                            let data = match parse_request_data::<T::Data>(request) {
                                Ok(d) => d,
                                Err(e) => return build_err(e),
                            };
                            CrudOperation::Create {
                                parent_id,
                                after,
                                data,
                            }
                        }
                    }
                }
            }
            &Method::GET => {
                if has_flag(request, "all") {
                    if !is_allowed_access(&metadata, &self.access.list) {
                        return build_err(UnauthorizedError::new());
                    }
                    let parent_id = match get_optional_pksk(request, "parent_id") {
                        Ok(v) => v,
                        Err(e) => return build_err(e),
                    };
                    CrudOperation::List { parent_id }
                } else if let Some(res) = maybe_ids(request) {
                    if !is_allowed_access(&metadata, &self.access.batch_read) {
                        return build_err(UnauthorizedError::new());
                    }
                    let ids = match res {
                        Ok(v) => v,
                        Err(e) => return build_err(e),
                    };
                    CrudOperation::ReadMultiple { ids }
                } else {
                    if !is_allowed_access(&metadata, &self.access.read) {
                        return build_err(UnauthorizedError::new());
                    }
                    let id = match get_required_id(request) {
                        Ok(id) => id,
                        Err(e) => return build_err(e),
                    };
                    CrudOperation::Read { id }
                }
            }
            &Method::PUT => {
                if !is_allowed_access(&metadata, &self.access.update) {
                    return build_err(UnauthorizedError::new());
                }
                let item = match parse_request_data::<T>(request) {
                    Ok(i) => i,
                    Err(e) => return build_err(e),
                };
                CrudOperation::Update { item }
            }
            &Method::DELETE => {
                let non_recursive = has_flag(request, "non_recursive");
                if non_recursive && !self.access.allow_non_recursive_delete {
                    return build_err(UnauthorizedError::new());
                }
                if has_flag(request, "all") {
                    if !is_allowed_access(&metadata, &self.access.delete_all) {
                        return build_err(UnauthorizedError::new());
                    }
                    let parent_id = match get_optional_pksk(request, "parent_id") {
                        Ok(v) => v,
                        Err(e) => return build_err(e),
                    };
                    CrudOperation::DeleteAll {
                        parent_id,
                        non_recursive,
                    }
                } else if let Some(res) = maybe_ids(request) {
                    if !is_allowed_access(&metadata, &self.access.batch_delete) {
                        return build_err(UnauthorizedError::new());
                    }
                    let ids = match res {
                        Ok(v) => v,
                        Err(e) => return build_err(e),
                    };
                    CrudOperation::DeleteMultiple { ids, non_recursive }
                } else {
                    if !is_allowed_access(&metadata, &self.access.delete) {
                        return build_err(UnauthorizedError::new());
                    }
                    let id = match get_required_id(request) {
                        Ok(id) => id,
                        Err(e) => return build_err(e),
                    };
                    CrudOperation::Delete { id, non_recursive }
                }
            }
            _ => return build_err(CriticalError::new("unsupported HTTP method for CRUD route")),
        };
        if let Err(e) = self.validation.validate(request, &op, &metadata) {
            return build_err(e);
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
    owner_of_id: Box<dyn Fn(&PkSk) -> Option<&str> + Send + Sync>,
    owner_of_parent_id: Box<dyn Fn(&PkSk) -> Option<&str> + Send + Sync>,
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
        FOwnerId: Fn(&PkSk) -> Option<&str> + Send + Sync + 'static,
        FOwnerParentId: Fn(&PkSk) -> Option<&str> + Send + Sync + 'static,
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
        let method = &request.http_method;
        let op = match method {
            &Method::POST => {
                if has_flag(request, "replace_all") {
                    if !preliminary_access_check(&metadata, &self.access.replace_all) {
                        return build_err(UnauthorizedError::new());
                    }
                    let parent_id = match get_optional_pksk(request, "parent_id") {
                        Ok(v) => v,
                        Err(e) => return build_err(e),
                    };
                    let authorized = match parent_id {
                        Some(ref pid) => is_allowed_owned_access(
                            &metadata,
                            &self.access.replace_all,
                            (self.owner_of_parent_id)(pid),
                        ),
                        None => is_allowed_owned_access(
                            &metadata,
                            &self.access.replace_all,
                            (self.owner_of_parent_id)(&PkSk::root()),
                        ),
                    };
                    if !authorized {
                        return build_err(UnauthorizedError::new());
                    }
                    let data = match parse_request_data::<Vec<T::Data>>(request) {
                        Ok(d) => d,
                        Err(e) => return build_err(e),
                    };
                    CrudOperation::ReplaceAll { parent_id, data }
                } else {
                    let parent_id = match get_optional_pksk(request, "parent_id") {
                        Ok(v) => v,
                        Err(e) => return build_err(e),
                    };
                    let after = match get_optional_pksk(request, "after") {
                        Ok(v) => v,
                        Err(e) => return build_err(e),
                    };
                    // Batch create if body is a list; fall back to single.
                    match parse_request_data::<Vec<T::Data>>(request) {
                        Ok(list) => {
                            if !preliminary_access_check(&metadata, &self.access.batch_create) {
                                return build_err(UnauthorizedError::new());
                            }
                            let authorized = match parent_id {
                                Some(ref pid) => is_allowed_owned_access(
                                    &metadata,
                                    &self.access.batch_create,
                                    (self.owner_of_parent_id)(pid),
                                ),
                                None => is_allowed_owned_access(
                                    &metadata,
                                    &self.access.batch_create,
                                    (self.owner_of_parent_id)(&PkSk::root()),
                                ),
                            };
                            if !authorized {
                                return build_err(UnauthorizedError::new());
                            }
                            CrudOperation::CreateMultiple {
                                parent_id,
                                after,
                                data: list,
                            }
                        }
                        Err(_) => {
                            if !preliminary_access_check(&metadata, &self.access.create) {
                                return build_err(UnauthorizedError::new());
                            }
                            let authorized = match parent_id {
                                Some(ref pid) => is_allowed_owned_access(
                                    &metadata,
                                    &self.access.create,
                                    (self.owner_of_parent_id)(pid),
                                ),
                                None => is_allowed_owned_access(
                                    &metadata,
                                    &self.access.create,
                                    (self.owner_of_parent_id)(&PkSk::root()),
                                ),
                            };
                            if !authorized {
                                return build_err(UnauthorizedError::new());
                            }
                            let data = match parse_request_data::<T::Data>(request) {
                                Ok(d) => d,
                                Err(e) => return build_err(e),
                            };
                            CrudOperation::Create {
                                parent_id,
                                after,
                                data,
                            }
                        }
                    }
                }
            }
            &Method::GET => {
                if has_flag(request, "all") {
                    if !preliminary_access_check(&metadata, &self.access.list) {
                        return build_err(UnauthorizedError::new());
                    }
                    let parent_id = match get_optional_pksk(request, "parent_id") {
                        Ok(v) => v,
                        Err(e) => return build_err(e),
                    };
                    let authorized = match parent_id.as_ref() {
                        Some(pid) => is_allowed_owned_access(
                            &metadata,
                            &self.access.list,
                            (self.owner_of_parent_id)(pid),
                        ),
                        None => is_allowed_owned_access(
                            &metadata,
                            &self.access.list,
                            (self.owner_of_parent_id)(&PkSk::root()),
                        ),
                    };
                    if !authorized {
                        return build_err(UnauthorizedError::new());
                    }
                    CrudOperation::List { parent_id }
                } else if let Some(res) = maybe_ids(request) {
                    if !preliminary_access_check(&metadata, &self.access.batch_read) {
                        return build_err(UnauthorizedError::new());
                    }
                    let ids = match res {
                        Ok(v) => v,
                        Err(e) => return build_err(e),
                    };
                    let all_authorized = ids.iter().all(|id| {
                        is_allowed_owned_access(
                            &metadata,
                            &self.access.batch_read,
                            (self.owner_of_id)(id),
                        )
                    });
                    if !all_authorized {
                        return build_err(UnauthorizedError::new());
                    }
                    CrudOperation::ReadMultiple { ids }
                } else {
                    if !preliminary_access_check(&metadata, &self.access.read) {
                        return build_err(UnauthorizedError::new());
                    }
                    let id = match get_required_id(request) {
                        Ok(id) => id,
                        Err(e) => return build_err(e),
                    };
                    let owner = (self.owner_of_id)(&id);
                    if !is_allowed_owned_access(&metadata, &self.access.read, owner) {
                        return build_err(UnauthorizedError::new());
                    }
                    CrudOperation::Read { id }
                }
            }
            &Method::PUT => {
                if !preliminary_access_check(&metadata, &self.access.update) {
                    return build_err(UnauthorizedError::new());
                }
                let item = match parse_request_data::<T>(request) {
                    Ok(i) => i,
                    Err(e) => return build_err(e),
                };
                let owner = (self.owner_of_id)(item.id());
                if !is_allowed_owned_access(&metadata, &self.access.update, owner) {
                    return build_err(UnauthorizedError::new());
                }
                CrudOperation::Update { item }
            }
            &Method::DELETE => {
                let non_recursive = has_flag(request, "non_recursive");
                if non_recursive && !self.access.allow_non_recursive_delete {
                    return build_err(UnauthorizedError::new());
                }
                if has_flag(request, "all") {
                    if !preliminary_access_check(&metadata, &self.access.delete_all) {
                        return build_err(UnauthorizedError::new());
                    }
                    let parent_id = match get_optional_pksk(request, "parent_id") {
                        Ok(v) => v,
                        Err(e) => return build_err(e),
                    };
                    let authorized = match parent_id {
                        Some(ref pid) => is_allowed_owned_access(
                            &metadata,
                            &self.access.delete_all,
                            (self.owner_of_parent_id)(pid),
                        ),
                        None => is_allowed_owned_access(
                            &metadata,
                            &self.access.delete_all,
                            (self.owner_of_parent_id)(&PkSk::root()),
                        ),
                    };
                    if !authorized {
                        return build_err(UnauthorizedError::new());
                    }
                    CrudOperation::DeleteAll {
                        parent_id,
                        non_recursive,
                    }
                } else if let Some(res) = maybe_ids(request) {
                    if !preliminary_access_check(&metadata, &self.access.batch_delete) {
                        return build_err(UnauthorizedError::new());
                    }
                    let ids = match res {
                        Ok(v) => v,
                        Err(e) => return build_err(e),
                    };
                    let all_authorized = ids.iter().all(|id| {
                        is_allowed_owned_access(
                            &metadata,
                            &self.access.batch_delete,
                            (self.owner_of_id)(id),
                        )
                    });
                    if !all_authorized {
                        return build_err(UnauthorizedError::new());
                    }
                    CrudOperation::DeleteMultiple { ids, non_recursive }
                } else {
                    if !preliminary_access_check(&metadata, &self.access.delete) {
                        return build_err(UnauthorizedError::new());
                    }
                    let id = match get_required_id(request) {
                        Ok(id) => id,
                        Err(e) => return build_err(e),
                    };
                    let owner = (self.owner_of_id)(&id);
                    if !is_allowed_owned_access(&metadata, &self.access.delete, owner) {
                        return build_err(UnauthorizedError::new());
                    }
                    CrudOperation::Delete { id, non_recursive }
                }
            }
            _ => return build_err(CriticalError::new("unsupported HTTP method for CRUD route")),
        };
        if let Err(e) = self.validation.validate(request, &op, &metadata) {
            return build_err(e);
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

fn has_flag(request: &ApiGatewayProxyRequest, key: &str) -> bool {
    request.query_string_parameters.first(key).is_some()
}

fn maybe_ids(request: &ApiGatewayProxyRequest) -> Option<Result<Vec<PkSk>, ServerError>> {
    request.query_string_parameters.first("ids").map(|raw| {
        let mut ids: Vec<PkSk> = Vec::new();
        if raw.trim().is_empty() {
            return Err(InvalidRequestError::new(&format!(
                "query parameter 'ids' must not be empty"
            )));
        }
        for part in raw.split(',') {
            let trimmed = part.trim();
            if trimmed.is_empty() {
                return Err(InvalidRequestError::new(&format!(
                    "query parameter 'ids' contains empty id"
                )));
            }
            match PkSk::from_string(trimmed) {
                Ok(p) => ids.push(p),
                Err(e) => return Err(InvalidRequestError::with_debug("invalid id in 'ids'", &e)),
            }
        }
        Ok(ids)
    })
}
