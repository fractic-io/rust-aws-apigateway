# fractic-aws-apigateway

The Lambda side of the Fractic APIs: macros that turn a function into an API
Gateway Lambda (`aws_lambda_handle_raw!`, `aws_lambda_handle_with_function!`,
`aws_lambda_handle_with_router!` with a `RoutingConfig` of `function_routes`
/ `crud_routes`), the route specs (`Function`, `NullaryFunction`,
`OwnedFunction`, `Crud`, `Access`, `Validation`, `AdminOnly`), request
parsing / auth, JSON response building and the standard API errors.
Consumer side: `/efs/fractic/_/rust.md` §3 and
`app/backend/_/adding_an_endpoint.md`.

`src/lib.rs` → `handle_raw/macros.rs`, `handle_with_function/macros.rs`,
`handle_with_router/{macros.rs, routing_config.rs,
std/{crud_specs,function_specs,validators}.rs}`,
`shared/{auth_utils,request_processing,response_building}.rs`,
`constants.rs`, `errors.rs`. `serde_json` is remapped to
`serde_json_path_to_error` crate-wide.

- Verify macro changes by building `app/backend/api/v1` through its
  `[patch]`; generated code may only use paths that resolve there
  (`fractic_*`, `serde_json`, `lambda_runtime`, `aws_lambda_events`).
- CORS headers are hard-coded for `https://fractic.io` in
  `response_building.rs`; the response shape is parsed by the Flutter
  `ApiUtil`, so changing it is a frontend change too.
- Commit / PR titles: `[crud] …`, `[router] …`.
