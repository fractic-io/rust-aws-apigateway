//! The Lambda's error sink: every `ServerError` constructed while handling
//! an event, and every panic, becomes one JSON `ErrorRecord` line on stdout,
//! which the function's log group receives next to the runtime's own lines.
//! The nightly error triage reads the records; the `ERROR` lines that
//! `build_err` prints for escaped errors stay, because the log group's alarm
//! counts those.

use std::{env, panic};

use fractic_server_error::{install_error_sink, ErrorOrigin, ErrorRecord};

/// Installed by `aws_lambda_handle_raw!` before the runtime starts; a second
/// call changes nothing. `source` is the consumer crate's name. The origin's
/// environment comes from `ENVIRONMENT`, its session from the Lambda's
/// function name and its version from `API_VERSION` (`v1`), falling back to
/// the consumer's crate version.
pub fn install_lambda_error_sink(source: &str, crate_version: &str) {
    let origin = ErrorOrigin {
        source: source.to_owned(),
        env: env::var("ENVIRONMENT").ok().filter(|v| !v.is_empty()),
        session: env::var("AWS_LAMBDA_FUNCTION_NAME").ok(),
        version: Some(
            env::var("API_VERSION")
                .ok()
                .filter(|v| !v.is_empty())
                .map(|v| format!("v{v}"))
                .unwrap_or_else(|| crate_version.to_owned()),
        ),
    };
    let sink_origin = origin.clone();
    if !install_error_sink(Box::new(move |event| {
        println!(
            "{}",
            ErrorRecord::from_event(&event, &sink_origin).to_json_line()
        );
    })) {
        return;
    }
    let previous = panic::take_hook();
    panic::set_hook(Box::new(move |info| {
        println!("{}", ErrorRecord::from_panic(info, &origin).to_json_line());
        previous(info);
    }));
}
