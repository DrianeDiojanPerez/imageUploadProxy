pub mod github;

use async_trait::async_trait;
use serde::Serialize;
use std::fmt;

/// The ways a storage backend can fail to keep or hand back an image.
#[derive(Debug)]
pub enum RepoError {
    TooLarge,
    NotFound,
    Upstream(String),
    Config(String),
}

impl fmt::Display for RepoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RepoError::TooLarge => write!(f, "image is larger than the storage limit"),
            RepoError::NotFound => write!(f, "no image is stored under that name"),
            RepoError::Upstream(message) => write!(f, "storage backend failed: {message}"),
            RepoError::Config(message) => write!(f, "storage backend is misconfigured: {message}"),
        }
    }
}

impl std::error::Error for RepoError {}

/// How much of a backend's api budget has gone, and when it comes back.
///
/// Everything is optional because a backend may not have a budget at all,
/// or may not have spoken to its provider yet in this window.
///
/// A provider may hand out more than one budget at a time, in which case
/// these describe whichever one is closest to running out, since that is the
/// one that will start turning requests away first.
#[derive(Debug, Default, Clone, Serialize)]
pub struct Usage {
    pub limit: Option<u64>,
    pub used: Option<u64>,
    pub remaining: Option<u64>,
    pub percent_used: Option<f64>,
    pub percent_remaining: Option<f64>,
    pub resets_at: Option<u64>,
    pub resets_at_utc: Option<String>,
    pub resets_in_seconds: Option<u64>,
    pub resets_in: Option<String>,
    pub calls_this_window: u64,
    pub calls_since_boot: u64,

    /// Which of the backend's budgets the headline numbers describe.
    pub resource: Option<String>,

    /// When the backend last told us any of this, so a reader can see how
    /// old the numbers are rather than having to trust them blind.
    pub last_call_at: Option<u64>,
    pub last_call_ago: Option<String>,

    /// Calls counted across every run, present only when they are being
    /// kept on disk.
    pub calls_total: Option<u64>,
}

/// An image read back out of storage, ready to be written to a response.
pub struct StoredImage {
    pub bytes: Vec<u8>,
    pub content_type: &'static str,
}

/// A place images can be written to and read back from.
///
/// Everything a backend needs to know about naming, encoding, and talking
/// to its provider stays behind this trait, so the HTTP layer only ever
/// hands over bytes and receives a URL or an image back.
#[async_trait]
pub trait ImageRepository {
    /// Store the given bytes under a fresh unique name and return the URL
    /// that image is served from.
    ///
    /// The extension is passed separately because the caller has already
    /// worked out the real image type from the request, and the backend
    /// should not have to sniff the bytes a second time.
    async fn store(&self, bytes: Vec<u8>, ext: &str) -> Result<String, RepoError>;

    /// Read back the image stored under the given name.
    ///
    /// The name is the last segment of the URL that store handed out, and
    /// the caller has already checked its shape, so a backend can use it
    /// to build a path without escaping it again.
    async fn fetch(&self, name: &str) -> Result<StoredImage, RepoError>;

    /// Remove the image stored under the given name.
    ///
    /// Deleting something that is already gone is reported as NotFound so a
    /// caller can tell the difference between a name it got wrong and a
    /// backend that refused the work.
    async fn delete(&self, name: &str) -> Result<(), RepoError>;

    /// What the backend has spent of whatever budget it works against.
    ///
    /// A backend with no such limit says nothing, and the endpoint reports
    /// that rather than inventing numbers for it.
    async fn usage(&self) -> Option<Usage> {
        None
    }
}

/// The type to serve a stored image as, keyed by the extension we gave it.
///
/// Only the four types the upload handler accepts can ever reach this, so
/// anything unknown means a bug rather than user input and falls back to a
/// type no browser will try to render.
pub fn content_type_for(ext: &str) -> &'static str {
    match ext {
        "png" => "image/png",
        "jpg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        _ => "application/octet-stream",
    }
}
