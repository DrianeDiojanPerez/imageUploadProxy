use std::sync::Arc;

use axum::extract::{Multipart, Path, State};
use axum::http::header::{CACHE_CONTROL, CONTENT_TYPE, ETAG, IF_NONE_MATCH};
use axum::http::HeaderMap;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Serialize;
use serde_json::json;
use uuid::Uuid;

use crate::config::{normalize_ext, Settings};
use crate::repository::{ImageRepository, RepoError};

#[derive(Clone)]
pub struct AppState {
    pub repository: Arc<dyn ImageRepository + Send + Sync>,
    pub settings: Arc<Settings>,
}

#[derive(Serialize)]
pub struct UploadResponse {
    pub url: String,
}

pub enum ApiError {
    MissingFile,
    UnsupportedType(String),
    TooLarge(usize),
    NotFound,
    Unsupported(String),
    Malformed(String),
    Storage(RepoError),
}

impl From<RepoError> for ApiError {
    fn from(error: RepoError) -> Self {
        ApiError::Storage(error)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, message) = match self {
            ApiError::MissingFile => (
                StatusCode::BAD_REQUEST,
                "the request carried no file field".to_string(),
            ),
            ApiError::UnsupportedType(allowed) => (
                StatusCode::BAD_REQUEST,
                format!("only {allowed} images are accepted"),
            ),
            ApiError::TooLarge(mb) => (
                StatusCode::PAYLOAD_TOO_LARGE,
                format!("the image is larger than {mb} MB"),
            ),
            ApiError::NotFound => (
                StatusCode::NOT_FOUND,
                "no image is stored under that name".to_string(),
            ),
            ApiError::Unsupported(message) => (StatusCode::NOT_IMPLEMENTED, message),
            ApiError::Malformed(message) => (StatusCode::BAD_REQUEST, message),

            // The upstream detail can carry the token scope or the repo name,
            // so it goes to the logs and the caller only learns that the
            // storage backend is the side that broke.
            ApiError::Storage(error) => {
                // A missing image or an oversized one is the caller's doing,
                // so only a real backend fault is worth an error line.
                if matches!(error, RepoError::Upstream(_) | RepoError::Config(_)) {
                    tracing::error!(%error, "the storage backend failed");
                }

                match error {
                    RepoError::TooLarge => (
                        StatusCode::PAYLOAD_TOO_LARGE,
                        "the image is larger than the storage limit".to_string(),
                    ),
                    RepoError::NotFound => (
                        StatusCode::NOT_FOUND,
                        "no image is stored under that name".to_string(),
                    ),
                    RepoError::Upstream(_) => (
                        StatusCode::BAD_GATEWAY,
                        "the storage backend refused the request".to_string(),
                    ),
                    RepoError::Config(_) => (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "the storage backend is misconfigured".to_string(),
                    ),
                }
            }
        };

        // The request line already carries the status, but not the reason we
        // turned it down, so the rejection is worth a line of its own. The
        // 5xx side was logged above with the upstream detail attached.
        if !status.is_server_error() {
            tracing::warn!(status = status.as_u16(), reason = %message, "request rejected");
        }

        (status, Json(json!({ "error": message }))).into_response()
    }
}

/// Take the first file field off the form and hand it to the repository.
pub async fn upload(
    State(state): State<AppState>,
    mut multipart: Multipart,
) -> Result<Response, ApiError> {
    let settings = &state.settings;

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| malformed(e, settings.max_upload_mb()))?
    {
        let content_type = field.content_type().map(str::to_string);
        let file_name = field.file_name().map(str::to_string);

        // Plain text parts have neither a filename nor a content type, so we
        // skip them and let a form send its own extra fields alongside.
        if content_type.is_none() && file_name.is_none() {
            continue;
        }

        let ext = extension_for(settings, content_type.as_deref(), file_name.as_deref())
            .ok_or_else(|| ApiError::UnsupportedType(settings.allowed_list()))?;

        let bytes = field
            .bytes()
            .await
            .map_err(|e| malformed(e, settings.max_upload_mb()))?;

        if bytes.len() > settings.max_upload_bytes {
            return Err(ApiError::TooLarge(settings.max_upload_mb()));
        }

        let url = state.repository.store(bytes.to_vec(), &ext).await?;

        // Only the shape of the upload is worth keeping. The bytes and the
        // name the caller gave the file stay out of the logs.
        tracing::info!(
            %ext,
            size = bytes.len(),
            content_type = content_type.as_deref().unwrap_or("none"),
            %url,
            "stored image"
        );

        return Ok((StatusCode::CREATED, Json(UploadResponse { url })).into_response());
    }

    Err(ApiError::MissingFile)
}

/// Read an image back out of storage and serve it.
///
/// This is the only way in once the repo is private, so the token stays on
/// the server and the caller never learns where the bytes actually live.
pub async fn serve(
    State(state): State<AppState>,
    Path(name): Path<String>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    // The name has to be exactly what store handed out. Checking it against
    // that shape is what stops a path like ../../.env from being read back
    // out of the repo, since nothing else escapes it later on.
    if !is_stored_name(&state.settings, &name) {
        return Err(ApiError::NotFound);
    }

    let tag = etag_for(&name);

    // The name carries a uuid and the bytes behind it never change, so a
    // caller holding this tag is holding the only version there will ever
    // be. Answering from the name alone means a revalidation costs us
    // nothing, not even the call to the storage backend.
    if is_known_to_caller(&headers, &tag) {
        tracing::debug!(%name, "answered a revalidation from the name alone");

        return Ok((
            StatusCode::NOT_MODIFIED,
            [
                (ETAG, tag.as_str()),
                (CACHE_CONTROL, state.settings.cache_control.as_str()),
            ],
        )
            .into_response());
    }

    let image = state.repository.fetch(&name).await?;

    tracing::info!(%name, size = image.bytes.len(), "served image");

    Ok((
        StatusCode::OK,
        [
            (CONTENT_TYPE, image.content_type),
            (CACHE_CONTROL, state.settings.cache_control.as_str()),
            (ETAG, tag.as_str()),
        ],
        image.bytes,
    )
        .into_response())
}

fn etag_for(name: &str) -> String {
    format!("\"{name}\"")
}

/// Whether the caller already holds the only version of this image.
///
/// If-None-Match can carry a list, and a cache is allowed to hand the tag
/// back marked weak, so the header is picked apart rather than compared
/// whole. A bare star means the caller will take anything it has.
fn is_known_to_caller(headers: &HeaderMap, tag: &str) -> bool {
    let Some(presented) = headers.get(IF_NONE_MATCH).and_then(|v| v.to_str().ok()) else {
        return false;
    };

    presented.split(',').any(|candidate| {
        let candidate = candidate.trim();

        candidate == "*" || candidate.trim_start_matches("W/") == tag
    })
}

/// Remove a stored image.
///
/// This one sits behind the upload secret, since anyone who can read a URL
/// would otherwise be able to throw the image away.
pub async fn remove(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Result<Response, ApiError> {
    if !is_stored_name(&state.settings, &name) {
        return Err(ApiError::NotFound);
    }

    state.repository.delete(&name).await?;

    tracing::info!(%name, "deleted image");

    Ok(StatusCode::NO_CONTENT.into_response())
}

/// Report how much of the backend's api budget is gone.
///
/// The numbers come from the backend's provider rather than from a tally we
/// keep ourselves, since the same token may well be in use somewhere else
/// and only the provider knows the real total.
pub async fn usage(State(state): State<AppState>) -> Result<Response, ApiError> {
    let Some(usage) = state.repository.usage().await else {
        return Err(ApiError::Unsupported(
            "this storage backend does not report api usage".to_string(),
        ));
    };

    let mut body = json!(usage);

    // The reset GitHub reports is UTC, which is rarely the clock the person
    // reading this is looking at, so a configured zone earns a second line
    // rather than replacing the one that came off the wire.
    if let (Some(zone), Some(resets_at)) = (&state.settings.timezone, usage.resets_at) {
        body["resets_at_local"] = json!(crate::usage::to_zone_string(resets_at, zone));
    }

    Ok(Json(body).into_response())
}

/// Whether a name is one this service could have handed out.
fn is_stored_name(settings: &Settings, name: &str) -> bool {
    let Some((stem, ext)) = name.rsplit_once('.') else {
        return false;
    };

    Uuid::parse_str(stem).is_ok() && settings.allowed_ext.iter().any(|allowed| allowed == ext)
}

/// Work out the stored extension from the declared type or the filename.
///
/// Browsers and curl are both happy to send an image with no content type at
/// all, so the filename is kept as a second chance before we give up on it.
fn extension_for(
    settings: &Settings,
    content_type: Option<&str>,
    file_name: Option<&str>,
) -> Option<String> {
    let from_type = content_type
        .and_then(|value| value.split(';').next())
        .map(|value| value.trim().to_ascii_lowercase())
        .and_then(|value| value.strip_prefix("image/").map(normalize_ext));

    let ext = match from_type {
        Some(ext) => ext,
        None => file_name
            .and_then(|name| name.rsplit_once('.'))
            .map(|(_, ext)| normalize_ext(ext))?,
    };

    settings
        .allowed_ext
        .iter()
        .any(|allowed| *allowed == ext)
        .then_some(ext)
}

// A body that runs past the router limit surfaces as a multipart error, so we
// read the status the extractor picked instead of calling everything a 400.
fn malformed(error: axum::extract::multipart::MultipartError, max_mb: usize) -> ApiError {
    if error.status() == StatusCode::PAYLOAD_TOO_LARGE {
        return ApiError::TooLarge(max_mb);
    }

    ApiError::Malformed(format!("could not read the multipart body: {error}"))
}
