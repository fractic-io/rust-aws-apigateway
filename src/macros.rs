#[macro_export]
macro_rules! aws_lambda {
    ($handler:expr) => {
        #[tokio::main]
        async fn main() -> Result<(), lambda_runtime::Error> {
            tracing_subscriber::fmt()
                .with_max_level(tracing::Level::INFO)
                // Disable printing module name in every log line.
                .with_target(false)
                // Disable printing time since CloudWatch already logs ingestion time.
                .without_time()
                .init();

            lambda_runtime::run(lambda_runtime::service_fn($handler)).await
        }
    };
}

#[macro_export]
macro_rules! aws_lambda_from_routing_config {
    ($config:expr) => {
        #[tokio::main]
        async fn main() -> Result<(), lambda_runtime::Error> {
            tracing_subscriber::fmt()
                .with_max_level(tracing::Level::INFO)
                // Disable printing module name in every log line.
                .with_target(false)
                // Disable printing time since CloudWatch already logs ingestion time.
                .without_time()
                .init();

            lambda_runtime::run(lambda_runtime::service_fn(|e| handle_route($config, e))).await
        }
    };
}

#[macro_export]
macro_rules! register_function_route {
    ($handler_name:ident, $func:ident, $validator:ident, $request_data_type:ident) => {
        pub async fn $handler_name(
            event: lambda_runtime::LambdaEvent<aws_lambda_events::apigw::ApiGatewayProxyRequest>,
            metadata: RequestMetadata,
        ) -> Result<aws_lambda_events::apigw::ApiGatewayProxyResponse, lambda_runtime::Error> {
            match parse_request_data::<$request_data_type>(&event.payload) {
                Ok(obj) => match $validator(&obj, metadata) {
                    Ok(_) => match $func(obj).await {
                        Ok(result) => build_result(result),
                        Err(func_error) => build_error(func_error),
                    },
                    Err(validation_error) => build_error(validation_error),
                },
                Err(request_parsing_error) => build_error(request_parsing_error),
            }
        }
    };
    ($handler_name:ident, $func:ident, $validator:ident) => {
        pub async fn $handler_name(
            event: lambda_runtime::LambdaEvent<aws_lambda_events::apigw::ApiGatewayProxyRequest>,
            metadata: RequestMetadata,
        ) -> Result<aws_lambda_events::apigw::ApiGatewayProxyResponse, lambda_runtime::Error> {
            match $validator(metadata) {
                Ok(_) => match $func().await {
                    Ok(result) => build_result(result),
                    Err(func_error) => build_error(func_error),
                },
                Err(validation_error) => build_error(validation_error),
            }
        }
    };
}

#[macro_export]
macro_rules! register_function_route_raw {
    ($handler_name:ident, $func:ident) => {
        pub async fn $handler_name(
            event: lambda_runtime::LambdaEvent<ApiGatewayProxyRequest>,
            metadata: RequestMetadata,
        ) -> Result<ApiGatewayProxyResponse, lambda_runtime::Error> {
            match $func(event.payload.headers, event.payload.body).await {
                Ok(result) => Ok(build_simple(result)),
                Err(func_error) => build_error(func_error),
            }
        }
    };
}

#[macro_export]
macro_rules! register_crud_route_from_scaffolding {
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
