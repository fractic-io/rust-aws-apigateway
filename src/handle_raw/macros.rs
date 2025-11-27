#[macro_export]
macro_rules! aws_lambda_handle_raw {
    ($handler:expr) => {
        #[::tokio::main]
        async fn main() -> Result<(), ::lambda_runtime::Error> {
            ::tracing_subscriber::fmt()
                .with_max_level(::tracing::Level::INFO)
                // Disable printing module name in every log line.
                .with_target(false)
                // Disable printing time since CloudWatch already logs ingestion time.
                .without_time()
                .init();

            ::lambda_runtime::run(::lambda_runtime::service_fn($handler)).await
        }
    };
}
