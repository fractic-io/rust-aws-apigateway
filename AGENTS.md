# fractic-aws-apigateway

The Lambda-side half of the Fractic API: macros that turn a function into an
API Gateway Lambda (`aws_lambda_handle_raw!`,
`aws_lambda_handle_with_function!`, `aws_lambda_handle_with_router!` with a
`RoutingConfig` of `function_routes` / `crud_routes`), the route specs
(`Function`, `NullaryFunction`, `OwnedFunction`, `Crud`, `Access`,
`Validation`, `AdminOnly`), request parsing / auth utilities, JSON response
building and the standard API errors (`UnauthorizedError`, …). Public,
provided as-is (`fractic-io/rust-aws-apigateway`, identity `fractic`).

## Read first

- `~/.agents/AGENTS.md` (workspace orientation; loaded automatically by Claude
  Code, read it if you are Codex or Grok) and `~/.agents/guides/rust.md`
  (§3 "Entry points" is the consumer side of these macros).
- `/efs/fractic/app/backend/_/adding_an_endpoint.md` for how a route is
  declared in practice.

## Layout

`src/lib.rs` is the module map: `handle_raw/macros.rs`,
`handle_with_function/macros.rs`, `handle_with_router/{macros.rs,
routing_config.rs, std/{crud_specs,function_specs,validators}.rs}`,
`shared/{auth_utils,request_processing,response_building}.rs`,
`constants.rs`, `errors.rs`. `serde_json` is remapped to
`serde_json_path_to_error` crate-wide for better parse errors.

## Working here

- No flake here: `nix develop /efs/fractic/storyengine/util --command cargo
  test`. Verify macro changes by building `app/backend/api/v1` through its
  `[patch]`.
- Macros expand in the consuming crate: generated code may only use paths that
  resolve there (`fractic_*`, `serde_json`, `lambda_runtime`,
  `aws_lambda_events`).
- CORS headers are hard-coded for `https://fractic.io` in
  `response_building.rs` (see README); the response shape is parsed by the
  Flutter `ApiUtil`, so changing it is a frontend change too.
- Commit / PR titles: `[crud] …`, `[router] …`, imperative, trailing period.
