use axum::extract::{Request, State};
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;

use crate::handlers::AppState;

pub const SECRET_HEADER: &str = "x-upload-secret";

/// Turn away callers that do not present the shared secret.
///
/// The layer is only mounted when UPLOAD_SECRET is set, so the state still
/// carries an Option and an unset secret means every request goes through.
pub async fn require_secret(State(state): State<AppState>, request: Request, next: Next) -> Response {
    let Some(expected) = state.settings.upload_secret.as_deref() else {
        return next.run(request).await;
    };

    let presented = request
        .headers()
        .get(SECRET_HEADER)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();

    if !secrets_match(expected, presented) {
        // The value that was presented is never logged, only the fact that
        // it did not match, so a near miss cannot be read back out.
        tracing::warn!(
            presented = !presented.is_empty(),
            "rejected an upload with a bad secret"
        );

        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({ "error": "a valid x-upload-secret header is required" })),
        )
            .into_response();
    }

    next.run(request).await
}

// Compare every byte even once we know it is wrong, so the time we take does
// not tell an attacker how much of the secret they have guessed so far.
fn secrets_match(expected: &str, presented: &str) -> bool {
    if expected.len() != presented.len() {
        return false;
    }

    expected
        .as_bytes()
        .iter()
        .zip(presented.as_bytes())
        .fold(0u8, |difference, (a, b)| difference | (a ^ b))
        == 0
}
