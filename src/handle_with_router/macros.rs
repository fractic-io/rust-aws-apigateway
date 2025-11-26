#[macro_export]
macro_rules! aws_lambda_handle_with_router {
    ($config:expr) => {
        $crate::aws_lambda_handle_raw!(move |e| $config.handle(e));
    };
}
