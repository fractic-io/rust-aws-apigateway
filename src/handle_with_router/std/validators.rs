use aws_lambda_events::apigw::ApiGatewayProxyRequest;
use fractic_server_error::ServerError;

use crate::{
    errors::UnauthorizedError, handle_with_router::routing_config::ValidatorSpec,
    shared::request_processing::RequestMetadata,
};

pub struct AdminOnly<I> {
    predicate: Box<dyn Fn(&I) -> bool + Send + Sync + 'static>,
}

impl<I: 'static> AdminOnly<I> {
    pub fn if_true<F>(predicate: F) -> Box<dyn ValidatorSpec<I> + 'static>
    where
        F: Fn(&I) -> bool + Send + Sync + 'static,
    {
        Box::new(Self {
            predicate: Box::new(predicate),
        })
    }
}

impl<I: 'static> ValidatorSpec<I> for AdminOnly<I> {
    fn validate(
        &self,
        _request: &ApiGatewayProxyRequest,
        data: &I,
        metadata: &RequestMetadata,
    ) -> Result<(), ServerError> {
        if (self.predicate)(data) {
            if metadata.is_authenticated && metadata.is_admin {
                Ok(())
            } else {
                Err(UnauthorizedError::new())
            }
        } else {
            Ok(())
        }
    }
}
