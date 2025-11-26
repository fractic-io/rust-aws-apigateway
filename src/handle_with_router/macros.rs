#[macro_export]
macro_rules! aws_lambda_handle_with_router {
    ($config:expr) => {
        $crate::aws_lambda_handle_raw!(move |e| $config.handle(e));
    };
}

macro_rules! tmp_register_crud_route_from_scaffolding {
    ($handler_name:ident, $ctx_arc:expr, $table_fn:expr, $type:ident) => {
        pub async fn $handler_name(
            event: LambdaEvent<ApiGatewayProxyRequest>,
            _: RequestMetadata,
        ) -> Result<ApiGatewayProxyResponse, Error> {
            async fn build_scaffolding(
            ) -> Result<CrudRouteScaffolding, ::fractic_server_error::ServerError> {
                let ctx: std::sync::Arc<_> = $ctx_arc;
                let table: String = $table_fn(&*ctx);
                CrudRouteScaffolding::new(&*ctx, table).await
            }
            match build_scaffolding().await {
                Ok(scaffolding) => scaffolding.handle_request::<$type>(event).await,
                Err(error) => build_error(error),
            }
        }
    };
}
