use async_trait::async_trait;
use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use reqwest::header::{ACCEPT, USER_AGENT};
use reqwest::StatusCode;
use serde_json::{json, Value};
use uuid::Uuid;

use super::{content_type_for, ImageRepository, RepoError, StoredImage, Usage};
use crate::usage::UsageTracker;

/// GitHub rejects API calls that arrive without a User-Agent header.
const CLIENT_USER_AGENT: &str = "image-upload-proxy";

/// The path segment images are served under.
///
/// This is the route, not the folder in the repo, so it stays put even when
/// GH_IMAGE_DIR moves the files somewhere else.
const URL_PREFIX: &str = "images";

#[derive(Clone, Debug)]
pub struct GitHubConfig {
    pub token: String,
    pub owner: String,
    pub repo: String,
    pub branch: String,
    pub image_dir: String,
    pub public_base_url: String,
    pub max_upload_bytes: usize,
    pub usage_file: Option<std::path::PathBuf>,
}

pub struct GitHubRepository {
    config: GitHubConfig,
    client: reqwest::Client,
    usage: UsageTracker,
}

impl GitHubRepository {
    /// Build a repository that keeps images in a repo through the Contents API.
    pub fn new(config: GitHubConfig) -> Result<Self, RepoError> {
        let client = reqwest::Client::builder()
            .build()
            .map_err(|e| RepoError::Config(format!("could not build the http client: {e}")))?;

        let usage = UsageTracker::new(config.usage_file.clone());

        Ok(Self {
            config,
            client,
            usage,
        })
    }

    /// Ask GitHub outright where the budget stands.
    ///
    /// This endpoint is the one call GitHub does not charge for, so it is
    /// safe to lean on when nothing else has happened lately and the numbers
    /// we are holding have gone cold.
    async fn refresh_usage(&self) -> Result<(), RepoError> {
        self.dispatch_as(
            self.request(
                self.client.get("https://api.github.com/rate_limit"),
                "application/vnd.github+json",
            ),
            false,
        )
        .await?;

        Ok(())
    }

    /// Send a request and note what it cost us.
    ///
    /// Every call to GitHub goes through here, which is the only reason the
    /// usage numbers can be trusted to cover all of them.
    async fn dispatch(&self, builder: reqwest::RequestBuilder) -> Result<reqwest::Response, RepoError> {
        self.dispatch_as(builder, true).await
    }

    async fn dispatch_as(
        &self,
        builder: reqwest::RequestBuilder,
        counted: bool,
    ) -> Result<reqwest::Response, RepoError> {
        let response = builder
            .send()
            .await
            .map_err(|e| RepoError::Upstream(format!("request to github failed: {e}")))?;

        self.usage.record(response.headers(), counted);

        Ok(response)
    }

    /// Send a request and read the json back, turning any refusal into a
    /// RepoError that carries what GitHub actually said.
    async fn send<T: serde::de::DeserializeOwned>(
        &self,
        builder: reqwest::RequestBuilder,
    ) -> Result<T, RepoError> {
        let response = self.dispatch(builder).await?;

        let status = response.status();

        if status == StatusCode::NOT_FOUND {
            return Err(RepoError::NotFound);
        }

        if !status.is_success() {
            let payload = response.text().await.unwrap_or_default();

            return Err(RepoError::Upstream(format!("github returned {status}: {payload}")));
        }

        response
            .json()
            .await
            .map_err(|e| RepoError::Upstream(format!("could not read the github response: {e}")))
    }

    fn contents_url(&self, path: &str) -> String {
        format!(
            "https://api.github.com/repos/{}/{}/contents/{}",
            self.config.owner, self.config.repo, path
        )
    }

    /// The URL we hand out, which points back at this service rather than at
    /// GitHub, so a private repo stays reachable through us and only us.
    fn public_url(&self, name: &str) -> String {
        format!("{}/{}/{}", self.config.public_base_url, URL_PREFIX, name)
    }

    /// Look up the sha of the blob at the given path, which is what the
    /// Contents API wants before it will delete anything.
    async fn blob_sha(&self, path: &str) -> Result<String, RepoError> {
        let payload: Value = self
            .send(
                self.request(
                    self.client.get(self.contents_url(path)),
                    "application/vnd.github+json",
                )
                .query(&[("ref", &self.config.branch)]),
            )
            .await?;

        payload["sha"]
            .as_str()
            .map(str::to_string)
            .ok_or_else(|| RepoError::Upstream("github sent no sha for the file".to_string()))
    }

    fn request(&self, builder: reqwest::RequestBuilder, accept: &str) -> reqwest::RequestBuilder {
        builder
            .header(USER_AGENT, CLIENT_USER_AGENT)
            .header(ACCEPT, accept)
            .header("X-GitHub-Api-Version", "2022-11-28")
            .bearer_auth(&self.config.token)
    }
}

#[async_trait]
impl ImageRepository for GitHubRepository {
    async fn store(&self, bytes: Vec<u8>, ext: &str) -> Result<String, RepoError> {
        if bytes.len() > self.config.max_upload_bytes {
            return Err(RepoError::TooLarge);
        }

        // A random name keeps two uploads of the same photo from landing on
        // the same path, which the Contents API would reject as a conflict
        // unless we first looked up the existing blob sha.
        let name = format!("{}.{}", Uuid::new_v4(), ext);
        let path = format!("{}/{name}", self.config.image_dir);

        tracing::debug!(%path, branch = %self.config.branch, "writing to github");

        let body = json!({
            "message": format!("upload {path}"),
            "content": BASE64.encode(&bytes),
            "branch": self.config.branch,
        });

        let response = self
            .dispatch(
                self.request(self.client.put(self.contents_url(&path)), "application/vnd.github+json")
                    .json(&body),
            )
            .await?;

        let status = response.status();

        if !status.is_success() {
            let payload = response.text().await.unwrap_or_default();

            return Err(RepoError::Upstream(format!("github returned {status}: {payload}")));
        }

        // The download_url in the reply is deliberately ignored. On a private
        // repo it carries a token that expires within minutes, so handing it
        // to a caller would give out a link that dies on its own.
        Ok(self.public_url(&name))
    }

    async fn fetch(&self, name: &str) -> Result<StoredImage, RepoError> {
        let path = format!("{}/{name}", self.config.image_dir);

        tracing::debug!(%path, branch = %self.config.branch, "reading from github");

        // The raw media type makes GitHub answer with the file itself rather
        // than a json envelope, which also lifts the 1 MB ceiling that the
        // base64 encoded reply carries.
        let response = self
            .dispatch(
                self.request(
                    self.client.get(self.contents_url(&path)),
                    "application/vnd.github.raw",
                )
                .query(&[("ref", &self.config.branch)]),
            )
            .await?;

        let status = response.status();

        if status == StatusCode::NOT_FOUND {
            return Err(RepoError::NotFound);
        }

        if !status.is_success() {
            let payload = response.text().await.unwrap_or_default();

            return Err(RepoError::Upstream(format!("github returned {status}: {payload}")));
        }

        let bytes = response
            .bytes()
            .await
            .map_err(|e| RepoError::Upstream(format!("could not read the image body: {e}")))?;

        let ext = name.rsplit_once('.').map(|(_, ext)| ext).unwrap_or_default();

        Ok(StoredImage {
            bytes: bytes.to_vec(),
            content_type: content_type_for(ext),
        })
    }

    async fn delete(&self, name: &str) -> Result<(), RepoError> {
        let path = format!("{}/{name}", self.config.image_dir);

        // A delete has to name the exact blob it means to remove, so the sha
        // is looked up first. That also settles whether the file is there at
        // all before we go writing a commit about it.
        let sha = self.blob_sha(&path).await?;

        tracing::debug!(%path, branch = %self.config.branch, "deleting from github");

        let body = json!({
            "message": format!("delete {path}"),
            "sha": sha,
            "branch": self.config.branch,
        });

        let response = self
            .dispatch(
                self.request(
                    self.client.delete(self.contents_url(&path)),
                    "application/vnd.github+json",
                )
                .json(&body),
            )
            .await?;

        let status = response.status();

        if !status.is_success() {
            let payload = response.text().await.unwrap_or_default();

            return Err(RepoError::Upstream(format!("github returned {status}: {payload}")));
        }

        Ok(())
    }

    async fn usage(&self) -> Option<Usage> {
        if self.usage.is_stale() {
            if let Err(error) = self.refresh_usage().await {
                tracing::warn!(%error, "could not refresh the rate limit from github");
            }
        }

        Some(self.usage.snapshot())
    }
}
