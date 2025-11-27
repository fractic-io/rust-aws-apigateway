#[macro_export]
macro_rules! aws_lambda_handle_with_router {
    ($config:expr) => {
        async fn __handler(
            event: ::lambda_runtime::LambdaEvent<
                ::aws_lambda_events::apigw::ApiGatewayProxyRequest,
            >,
        ) -> Result<::aws_lambda_events::apigw::ApiGatewayProxyResponse, ::lambda_runtime::Error> {
            let config = $config;
            config.handle(event).await
        }
        $crate::aws_lambda_handle_raw!(__handler);
    };
}
