#[macro_export]
macro_rules! aws_lambda_handle_with_function {
    ($validator:ident, $func:ident, $request_data_type:ident) => {
        async fn __handler(
            event: lambda_runtime::LambdaEvent<aws_lambda_events::apigw::ApiGatewayProxyRequest>,
        ) -> Result<aws_lambda_events::apigw::ApiGatewayProxyResponse, lambda_runtime::Error> {
            let metadata = match parse_request_metadata(&event.payload) {
                Ok(m) => m,
                Err(e) => return build_error(e),
            };
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
        $crate::aws_lambda_handle_raw!(__handler);
    };
    ($validator:ident, $func:ident) => {
        async fn __handler(
            event: lambda_runtime::LambdaEvent<aws_lambda_events::apigw::ApiGatewayProxyRequest>,
        ) -> Result<aws_lambda_events::apigw::ApiGatewayProxyResponse, lambda_runtime::Error> {
            let metadata = match parse_request_metadata(&event.payload) {
                Ok(m) => m,
                Err(e) => return build_error(e),
            };
            match $validator(metadata) {
                Ok(_) => match $func().await {
                    Ok(result) => build_result(result),
                    Err(func_error) => build_error(func_error),
                },
                Err(validation_error) => build_error(validation_error),
            }
        }
        $crate::aws_lambda_handle_raw!(__handler);
    };
    (unwrapped $func:ident) => {
        async fn __handler(
            event: lambda_runtime::LambdaEvent<ApiGatewayProxyRequest>,
        ) -> Result<ApiGatewayProxyResponse, lambda_runtime::Error> {
            match $func(event.payload.headers, event.payload.body).await {
                Ok(result) => Ok(build_simple(result)),
                Err(func_error) => build_error(func_error),
            }
        }
        $crate::aws_lambda_handle_raw!(__handler);
    };
}
