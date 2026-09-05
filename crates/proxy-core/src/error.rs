//! Structured error bodies for all proxy endpoints
//! ([`schemas/error.jtd.json`](../../../schemas/error.jtd.json)).
//!
//! Errors never contain API key material, request bodies, or response
//! bodies. Messages are operator-facing and MUST be treated as untrusted
//! input by the harness.

use serde::Deserialize;
use serde::Serialize;

/// Machine-readable error code. The HTTP status is derived from the code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    ModelNotEnabled,
    ServiceDisabled,
    Forbidden,
    NotFound,
    BadRequest,
    UpstreamError,
    Internal,
}

impl ErrorCode {
    /// HTTP status for this error code, per the spec's error semantics.
    pub fn http_status(self) -> u16 {
        match self {
            Self::ModelNotEnabled | Self::ServiceDisabled | Self::Forbidden => 403,
            Self::NotFound => 404,
            Self::BadRequest => 400,
            Self::UpstreamError => 502,
            Self::Internal => 500,
        }
    }
}

/// A single structured error, as served in the `error` field of every
/// error response body.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ErrorDetail {
    pub code: ErrorCode,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub service_id: Option<String>,
}

/// Error response body: `{"error":{"code":…,"message":…}}`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ErrorDocument {
    pub error: ErrorDetail,
}

impl ErrorDocument {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            error: ErrorDetail {
                code,
                message: message.into(),
                model: None,
                service_id: None,
            },
        }
    }

    /// The lockdown refusal: the request named a model that is not present,
    /// allowed, and enabled.
    pub fn model_not_enabled(model: impl Into<String>) -> Self {
        let model = model.into();
        let mut doc = Self::new(
            ErrorCode::ModelNotEnabled,
            format!("model {model} is not present, allowed, and enabled"),
        );
        doc.error.model = Some(model);
        doc
    }

    /// The service gate refusal: key absent or operator-disabled.
    pub fn service_disabled(service_id: impl Into<String>) -> Self {
        let service_id = service_id.into();
        let mut doc = Self::new(
            ErrorCode::ServiceDisabled,
            format!("service {service_id} is not enabled"),
        );
        doc.error.service_id = Some(service_id);
        doc
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn error_document_round_trips() {
        let doc = ErrorDocument::model_not_enabled("glm-5-2");
        let json = serde_json::to_string(&doc).expect("serialize");
        assert_eq!(
            json,
            r#"{"error":{"code":"model_not_enabled","message":"model glm-5-2 is not present, allowed, and enabled","model":"glm-5-2"}}"#
        );
        let reparsed: ErrorDocument = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(reparsed, doc);
    }

    #[test]
    fn lockdown_and_service_errors_are_forbidden() {
        assert_eq!(ErrorCode::ModelNotEnabled.http_status(), 403);
        assert_eq!(ErrorCode::ServiceDisabled.http_status(), 403);
    }

    #[test]
    fn status_codes_match_spec() {
        assert_eq!(ErrorCode::Forbidden.http_status(), 403);
        assert_eq!(ErrorCode::NotFound.http_status(), 404);
        assert_eq!(ErrorCode::BadRequest.http_status(), 400);
        assert_eq!(ErrorCode::UpstreamError.http_status(), 502);
        assert_eq!(ErrorCode::Internal.http_status(), 500);
    }
}
