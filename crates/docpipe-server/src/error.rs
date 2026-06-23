//! DocError → axum 响应（kebab-code + spec §7 HTTP 状态）。

use docpipe_core::error::DocError;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;

pub struct ApiError(pub DocError);

impl From<DocError> for ApiError {
    fn from(e: DocError) -> Self {
        ApiError(e)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = StatusCode::from_u16(self.0.http_status())
            .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        let body =
            Json(serde_json::json!({ "error": self.0.code(), "detail": self.0.to_string() }));
        (status, body).into_response()
    }
}
