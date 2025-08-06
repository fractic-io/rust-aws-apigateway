use aws_lambda_events::{
    apigw::ApiGatewayProxyResponse,
    encodings::Body,
    http::{
        header::{
            ACCESS_CONTROL_ALLOW_CREDENTIALS, ACCESS_CONTROL_ALLOW_HEADERS,
            ACCESS_CONTROL_ALLOW_METHODS, ACCESS_CONTROL_ALLOW_ORIGIN,
        },
        HeaderMap,
    },
};
use fractic_server_error::ServerError;
use lambda_runtime::Error;
use serde::Serialize;

use crate::constants::{INTERNAL_SERVER_ERROR_MSG, UNAUTHORIZED_ERROR_MSG};

// API Gateway response utils.
// --------------------------------------------------

// All API responses are wrapped in the following wrapper:
#[derive(Debug, Serialize)]
struct ResponseWrapper {
    ok: bool,
    // If OK, response data.
    data: Option<String>,
    // If not OK, error message safe to show to user.
    error: Option<String>,
}

pub fn build_simple(data: impl Into<Body>) -> ApiGatewayProxyResponse {
    ApiGatewayProxyResponse {
        status_code: 200,
        headers: build_headers(),
        multi_value_headers: Default::default(),
        body: Some(data.into()),
        is_base64_encoded: false,
    }
}

pub fn build_result<T>(data: T) -> Result<ApiGatewayProxyResponse, Error>
where
    T: serde::Serialize,
{
    let payload = ResponseWrapper {
        ok: true,
        data: Some(serde_json::to_string(&data)?),
        error: None,
    };
    let resp = ApiGatewayProxyResponse {
        status_code: 200,
        headers: build_headers(),
        multi_value_headers: Default::default(),
        body: Some(serde_json::to_string(&payload)?.into()),
        is_base64_encoded: false,
    };
    Ok(resp)
}

pub fn build_error(error: ServerError) -> Result<ApiGatewayProxyResponse, Error> {
    enum LoggingLevel {
        Error,
        Warning,
        Info,
    }

    // Two ways to handle errors:

    // 1) Forward to the client by wrapping the error in a 200 response. This
    // allows the client to gracefully handle it.
    let forward_to_client = |public_msg: &str, logging_level: LoggingLevel| {
        match logging_level {
            LoggingLevel::Error => eprintln!("ERROR\n{}", error),
            LoggingLevel::Warning => println!("WARNING\n{}", error),
            LoggingLevel::Info => println!("INFO\n{}", error),
        }
        println!("NOTE: Forwarding error to client. Returning 200 response.");
        // Since the data field will be set to None, we need to specify the
        // correct type T, so just use int.
        let payload = ResponseWrapper {
            ok: false,
            data: None,
            error: Some(public_msg.into()),
        };
        Ok::<_, Error>(ApiGatewayProxyResponse {
            // Outer status code should still be 200 for client-errors,
            // otherwise Amplify will treat it as a server error. The client
            // will know there is a client error because ok == false.
            status_code: 200,
            headers: build_headers(),
            multi_value_headers: Default::default(),
            body: Some(serde_json::to_string(&payload)?.into()),
            is_base64_encoded: false,
        })
    };

    // 2) Return an error response, triggerring alerting, affecting lambda
    // statistics, and avoiding leaking any error data to the client.
    let error_response = |error_code: i64, public_msg: &str| {
        eprintln!("ERROR\n{}", error);
        Ok::<_, Error>(ApiGatewayProxyResponse {
            status_code: error_code,
            headers: build_headers(),
            multi_value_headers: Default::default(),
            body: Some(public_msg.into()),
            is_base64_encoded: false,
        })
    };

    // Decide based on the error behaviour type.
    match error.behaviour() {
        fractic_server_error::ServerErrorBehaviour::ForwardToClient => {
            forward_to_client(error.message(), LoggingLevel::Info)
        }
        fractic_server_error::ServerErrorBehaviour::LogWarningForwardToClient => {
            forward_to_client(error.message(), LoggingLevel::Warning)
        }
        fractic_server_error::ServerErrorBehaviour::LogErrorForwardToClient => {
            forward_to_client(error.message(), LoggingLevel::Error)
        }
        fractic_server_error::ServerErrorBehaviour::LogWarningSendFixedMsgToClient(fixed_msg) => {
            forward_to_client(fixed_msg, LoggingLevel::Warning)
        }
        fractic_server_error::ServerErrorBehaviour::LogErrorSendFixedMsgToClient(fixed_msg) => {
            forward_to_client(fixed_msg, LoggingLevel::Error)
        }
        fractic_server_error::ServerErrorBehaviour::ReturnInternalServerError => {
            error_response(500, INTERNAL_SERVER_ERROR_MSG)
        }
        fractic_server_error::ServerErrorBehaviour::ReturnUnauthorized => {
            error_response(401, UNAUTHORIZED_ERROR_MSG)
        }
    }
}

// Helper functions.
// --------------------------------------------------

fn build_headers() -> HeaderMap {
    let mut headers = HeaderMap::new();
    //
    // Build CORS headers to support web clients hosted on https://fractic.io
    // accessing the API.
    //
    // Most modern browsers will not allow a web client to make a request to an
    // API unless the relevant CORS headers are set.
    //
    // NOTE: In addition to requiring the proper response headers on the request
    // itself, most modern browsers also make preflight OPTION requests before
    // sending the actual API request. These preflight requests should be
    // handled separately, and should also respond with the same CORS response
    // headers as we do here (and no body). Those preflight handlers can be
    // auto-generated by API Gateway by configuring the 'Cors' property on the
    // AWS::Serverless::Api resource:
    //
    //   Cors:
    //     AllowMethods: "'GET, POST, PUT, DELETE'"
    //     AllowHeaders: "'Content-Type,X-Amz-Date,Authorization,X-Api-Key,X-Amz-Security-Token,X-Amz-User-Agent'"
    //     AllowOrigin: "'https://example.com'"
    //     MaxAge: "'600'"
    //     AllowCredentials: true
    //   Auth:
    //     AddApiKeyRequiredToCorsPreflight: false
    //     AddDefaultAuthorizerToCorsPreflight: false
    //
    headers.insert(
        ACCESS_CONTROL_ALLOW_ORIGIN,
        "https://fractic.io".parse().unwrap(),
    );
    headers.insert(
        ACCESS_CONTROL_ALLOW_HEADERS,
        "Content-Type,X-Amz-Date,Authorization,X-Api-Key,X-Amz-Security-Token,X-Amz-User-Agent"
            .parse()
            .unwrap(),
    );
    headers.insert(
        ACCESS_CONTROL_ALLOW_METHODS,
        "GET, POST, PUT, DELETE".parse().unwrap(),
    );
    headers.insert(ACCESS_CONTROL_ALLOW_CREDENTIALS, "true".parse().unwrap());
    headers
}

// Tests.
// --------------------------------------------------

#[cfg(test)]
mod tests {
    use crate::UnauthorizedError;

    use super::*;
    use aws_lambda_events::encodings::Body;
    use fractic_server_error::{define_client_error, define_user_error, CriticalError};
    use serde::Deserialize;
    use serde_json::Value;

    #[derive(Debug, Serialize, Deserialize)]
    struct MockResponseData {
        key: String,
    }

    #[test]
    fn test_build_result_string() {
        let data = "Test string.".to_string();
        let result = build_result(data).unwrap();
        let body: Value = serde_json::from_str(match &result.body.unwrap() {
            Body::Text(b) => b,
            _ => panic!("Expected response body."),
        })
        .unwrap();

        assert_eq!(result.status_code, 200);
        assert_eq!(body["ok"].as_bool().unwrap(), true);
        assert_eq!(body["data"].as_str().unwrap(), "\"Test string.\"");
        assert_eq!(body["error"].is_null(), true);
    }

    #[test]
    fn test_build_result_object() {
        let error = MockResponseData {
            key: "Test value.".to_string(),
        };
        let result = build_result(error).unwrap();
        let body: Value = serde_json::from_str(match &result.body.unwrap() {
            Body::Text(b) => b,
            _ => panic!("Expected response body."),
        })
        .unwrap();

        assert_eq!(result.status_code, 200);
        assert_eq!(body["ok"].as_bool().unwrap(), true);
        assert_eq!(
            serde_json::from_str::<MockResponseData>(&body["data"].as_str().unwrap())
                .unwrap()
                .key,
            "Test value."
        );
        assert_eq!(body["error"].is_null(), true);
    }

    #[test]
    fn test_build_user_error() {
        define_user_error!(TestError, "User error: {details}.", { details: &str });
        let error = TestError::new("test details");
        let result = build_error(error).unwrap();
        let body: Value = serde_json::from_str(match &result.body.unwrap() {
            Body::Text(b) => b,
            _ => panic!("Expected response body."),
        })
        .unwrap();

        assert_eq!(result.status_code, 200);
        assert_eq!(body["ok"].as_bool().unwrap(), false);
        assert_eq!(body["data"].is_null(), true);
        assert!(body["error"]
            .as_str()
            .unwrap()
            .contains("User error: test details."));
    }

    #[test]
    fn test_build_client_error() {
        define_client_error!(TestError, "Client error: {details}.", { details: &str });
        let error = TestError::new("test details");
        let result = build_error(error).unwrap();
        let body: Value = serde_json::from_str(match &result.body.unwrap() {
            Body::Text(b) => b,
            _ => panic!("Expected response body."),
        })
        .unwrap();

        assert_eq!(result.status_code, 200);
        assert_eq!(body["ok"].as_bool().unwrap(), false);
        assert_eq!(body["data"].is_null(), true);
        assert!(body["error"]
            .as_str()
            .unwrap()
            .contains("An invalid request was made by the application."));
        assert!(!body["error"]
            .as_str()
            .unwrap()
            .to_lowercase()
            .contains("client error"));
    }

    #[test]
    fn test_build_internal_error() {
        let error = CriticalError::new("internal error message");
        let result = build_error(error).unwrap();
        let body = match result.body.unwrap() {
            Body::Text(b) => b,
            _ => panic!("Expected response body."),
        };
        assert_eq!(result.status_code, 500);
        assert!(!body.contains("internal error message"));
    }

    #[test]
    fn test_build_unauthorized_error() {
        let error =
            UnauthorizedError::with_debug(&"internal authentication error message".to_string());
        let result = build_error(error).unwrap();
        let body = match result.body.unwrap() {
            Body::Text(b) => b,
            _ => panic!("Expected response body."),
        };
        assert_eq!(result.status_code, 401);
        assert!(!body.contains("internal authentication error message"));
    }
}
