//! The Lambda's error sink: every `ServerError` constructed while handling
//! an event, and every panic, becomes one JSON `ErrorRecord` line on stdout,
//! which the function's log group receives next to the runtime's own lines.
//! Each record carries the id of the invocation it occurred in. These
//! records are the function's only error output.

use std::{
    env, panic,
    sync::{Mutex, OnceLock},
};

use fractic_server_error::{install_error_sink, ErrorOrigin, ErrorRecord};

// Public interface.
// ----------------------------------------------------------------------------

/// Installed by `aws_lambda_handle_raw!` before the runtime starts; a second
/// call changes nothing. `source` is the consumer crate's name. The origin's
/// environment comes from `ENVIRONMENT`, its entry from the Lambda's
/// function name and its version from `API_VERSION` (`v1`), falling back to
/// the consumer's crate version.
pub fn install_lambda_error_sink(source: &str, crate_version: &str) {
    let origin = ErrorOrigin {
        source: source.to_owned(),
        env: env::var("ENVIRONMENT").ok().filter(|v| !v.is_empty()),
        entry: env::var("AWS_LAMBDA_FUNCTION_NAME").ok(),
        version: Some(
            env::var("API_VERSION")
                .ok()
                .filter(|v| !v.is_empty())
                .map(|v| format!("v{v}"))
                .unwrap_or_else(|| crate_version.to_owned()),
        ),
    };
    if ORIGIN.set(origin).is_err() {
        return;
    }
    // Another sink installed first keeps the process; this one then adds no
    // panic hook either.
    if !install_error_sink(Box::new(|event| {
        if let Some(origin) = ORIGIN.get() {
            print_record(ErrorRecord::from_event(&event, origin));
        }
    })) {
        return;
    }
    let previous = panic::take_hook();
    panic::set_hook(Box::new(move |info| {
        if let Some(origin) = ORIGIN.get() {
            print_record(ErrorRecord::from_panic(info, origin));
        }
        previous(info);
    }));
}

/// Runs one invocation with its request id stamped on the records written
/// meanwhile. `lambda_runtime::run` handles one invocation at a time, so one
/// process-wide id is enough. Called by `aws_lambda_handle_raw!`.
#[doc(hidden)]
pub async fn __in_request<F, T>(request_id: String, invocation: F) -> T
where
    F: std::future::Future<Output = T>,
{
    set_request(Some(request_id));
    let output = invocation.await;
    set_request(None);
    output
}

// Internal.
// ----------------------------------------------------------------------------

static ORIGIN: OnceLock<ErrorOrigin> = OnceLock::new();
static REQUEST: Mutex<Option<String>> = Mutex::new(None);

fn print_record(mut record: ErrorRecord) {
    record.request = current_request();
    println!("{}", record.to_json_line());
}

// A poisoned lock only means a panic happened while it was held; the id in
// it is still the right one.
fn set_request(request_id: Option<String>) {
    *REQUEST.lock().unwrap_or_else(|e| e.into_inner()) = request_id;
}

fn current_request() -> Option<String> {
    REQUEST.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

// Tests.
// ----------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::{
        future::Future as _,
        pin::pin,
        task::{Context, Poll, Waker},
    };

    use super::*;

    #[test]
    fn an_invocation_sets_its_request_id_and_clears_it_after() {
        let invocation = __in_request("req-1".to_owned(), async { current_request() });
        let mut invocation = pin!(invocation);
        let seen = match invocation
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop()))
        {
            Poll::Ready(seen) => seen,
            Poll::Pending => panic!("the invocation awaits nothing"),
        };
        assert_eq!(seen.as_deref(), Some("req-1"));
        assert_eq!(current_request(), None);
    }
}
