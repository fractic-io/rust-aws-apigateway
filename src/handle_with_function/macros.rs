#[macro_export]
macro_rules! aws_lambda_handle_with_function {
    ($validator:path, $func:path, $request_data_type:ty) => {
        async fn __handler(
            event: ::lambda_runtime::LambdaEvent<
                ::aws_lambda_events::apigw::ApiGatewayProxyRequest,
            >,
        ) -> Result<::aws_lambda_events::apigw::ApiGatewayProxyResponse, ::lambda_runtime::Error> {
            let metadata = match $crate::parse_request_metadata(&event.payload) {
                Ok(m) => m,
                e @ Err(_) => return $crate::build_result(e),
            };
            match $crate::parse_request_data::<$request_data_type>(&event.payload) {
                Ok(obj) => match $validator(&obj, metadata) {
                    Ok(_) => $crate::build_result($func(obj).await),
                    e @ Err(_) => $crate::build_result(e),
                },
                e @ Err(_) => $crate::build_result(e),
            }
        }
        $crate::aws_lambda_handle_raw!(__handler);
    };
    ($validator:path, $func:path) => {
        async fn __handler(
            event: ::lambda_runtime::LambdaEvent<
                ::aws_lambda_events::apigw::ApiGatewayProxyRequest,
            >,
        ) -> Result<::aws_lambda_events::apigw::ApiGatewayProxyResponse, ::lambda_runtime::Error> {
            let metadata = match $crate::parse_request_metadata(&event.payload) {
                Ok(m) => m,
                e @ Err(_) => return $crate::build_result(e),
            };
            match $validator(metadata) {
                Ok(_) => $crate::build_result($func().await),
                e @ Err(_) => $crate::build_result(e),
            }
        }
        $crate::aws_lambda_handle_raw!(__handler);
    };
    (RAW $func:path) => {
        async fn __handler(
            event: ::lambda_runtime::LambdaEvent<
                ::aws_lambda_events::apigw::ApiGatewayProxyRequest,
            >,
        ) -> Result<::aws_lambda_events::apigw::ApiGatewayProxyResponse, ::lambda_runtime::Error> {
            match $func(event.payload.headers, event.payload.body).await {
                Ok(result) => Ok($crate::build_simple(result)),
                e @ Err(_) => $crate::build_result(e),
            }
        }
        $crate::aws_lambda_handle_raw!(__handler);
    };
}
