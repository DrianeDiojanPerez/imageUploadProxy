use std::env;

use jiff::tz::TimeZone;

use crate::repository::github::GitHubConfig;

/// The image types this service knows how to serve back.
///
/// ALLOWED_TYPES can narrow this list but not widen it, since anything else
/// would be stored with a content type no browser would render.
const SUPPORTED_TYPES: [&str; 4] = ["png", "jpg", "gif", "webp"];

pub struct Config {
    pub github: GitHubConfig,
    pub settings: Settings,
    pub bind_address: String,
}

/// The knobs the HTTP layer reads on the way through a request.
pub struct Settings {
    pub upload_secret: Option<String>,
    pub max_upload_bytes: usize,
    pub allowed_ext: Vec<String>,
    pub cache_control: String,
    pub timezone: Option<TimeZone>,
}

impl Config {
    /// Read the whole configuration from the environment.
    ///
    /// Anything the service cannot run without is read eagerly here so a bad
    /// deploy dies on boot instead of on the first upload someone tries.
    pub fn from_env() -> Self {
        let max_upload_bytes = max_upload_bytes();

        Self {
            github: GitHubConfig {
                token: required("GH_TOKEN"),
                owner: required("GH_OWNER"),
                repo: required("GH_REPO"),
                branch: optional("GH_BRANCH").unwrap_or_else(|| "main".to_string()),
                image_dir: image_dir(),
                public_base_url: public_base_url(),
                max_upload_bytes,
                usage_file: usage_file(),
            },
            settings: Settings {
                upload_secret: optional("UPLOAD_SECRET"),
                max_upload_bytes,
                allowed_ext: allowed_ext(),
                cache_control: cache_control(),
                timezone: timezone(),
            },
            bind_address: optional("BIND_ADDRESS").unwrap_or_else(|| "0.0.0.0:3000".to_string()),
        }
    }
}

impl Settings {
    /// The accepted types spelled out the way an error message wants them.
    pub fn allowed_list(&self) -> String {
        self.allowed_ext.join(", ")
    }

    pub fn max_upload_mb(&self) -> usize {
        self.max_upload_bytes / (1024 * 1024)
    }
}

/// Read TIMEZONE, which is how a reader says where they are.
///
/// A zone rather than an offset, so a place that moves its clocks for the
/// summer is read right in both halves of the year without anyone having to
/// remember to change this.
fn timezone() -> Option<TimeZone> {
    let name = optional("TIMEZONE")?;

    // The zone database is compiled in, so a wrong name is a typo rather
    // than a missing package, and saying so on boot beats an endpoint that
    // quietly shows the wrong hour.
    let zone = TimeZone::get(&name).unwrap_or_else(|_| {
        panic!("TIMEZONE must be an IANA zone name such as America/Belize, got {name}")
    });

    Some(zone)
}

// Kept on disk unless someone deliberately empties the variable, since a
// count that quietly starts over on every restart is worth less than one
// that does not.
fn usage_file() -> Option<std::path::PathBuf> {
    if env::var("USAGE_FILE").is_ok_and(|value| value.trim().is_empty()) {
        return None;
    }

    Some(std::path::PathBuf::from(
        optional("USAGE_FILE").unwrap_or_else(|| "./usage.json".to_string()),
    ))
}

fn max_upload_bytes() -> usize {
    let mb = number("MAX_UPLOAD_MB", 25);

    // GitHub itself gives up somewhere past 100 MB on the Contents API, and
    // a zero here would turn every upload into a 413.
    if mb == 0 || mb > 100 {
        panic!("MAX_UPLOAD_MB must be between 1 and 100, got {mb}");
    }

    mb * 1024 * 1024
}

fn allowed_ext() -> Vec<String> {
    let Some(configured) = optional("ALLOWED_TYPES") else {
        return SUPPORTED_TYPES.iter().map(|ext| ext.to_string()).collect();
    };

    let mut allowed: Vec<String> = Vec::new();

    for entry in configured.split(',') {
        let ext = normalize_ext(entry.trim());

        if ext.is_empty() {
            continue;
        }

        if !SUPPORTED_TYPES.contains(&ext.as_str()) {
            panic!(
                "ALLOWED_TYPES only understands {}, got {ext}",
                SUPPORTED_TYPES.join(", ")
            );
        }

        if !allowed.contains(&ext) {
            allowed.push(ext);
        }
    }

    if allowed.is_empty() {
        panic!("ALLOWED_TYPES was set but named no usable type");
    }

    allowed
}

// Both spellings of jpeg mean the same picture, and everything downstream
// works in the one the file is stored under.
pub fn normalize_ext(ext: &str) -> String {
    match ext.to_ascii_lowercase().as_str() {
        "jpeg" => "jpg".to_string(),
        other => other.to_string(),
    }
}

// Stored names carry a uuid, so the bytes behind one can never change and a
// long life is safe. Zero turns caching off for anyone debugging.
fn cache_control() -> String {
    match number("CACHE_MAX_AGE", 31536000) {
        0 => "no-store".to_string(),
        seconds => format!("public, max-age={seconds}, immutable"),
    }
}

// The route is always /images/{name}, this is only where the files sit
// inside the repo.
fn image_dir() -> String {
    optional("GH_IMAGE_DIR")
        .unwrap_or_else(|| "images".to_string())
        .trim_matches('/')
        .to_string()
}

// Every URL we hand out is built from this, so a trailing slash would show
// up in the middle of it and follow the image around forever.
fn public_base_url() -> String {
    optional("PUBLIC_BASE_URL")
        .unwrap_or_else(|| "http://localhost:3000".to_string())
        .trim_end_matches('/')
        .to_string()
}

fn number(key: &str, fallback: usize) -> usize {
    let Some(value) = optional(key) else {
        return fallback;
    };

    value
        .parse()
        .unwrap_or_else(|_| panic!("{key} must be a whole number, got {value}"))
}

fn required(key: &str) -> String {
    optional(key).unwrap_or_else(|| panic!("missing required env var {key}"))
}

// An env var that is set but empty is almost always a broken deploy script,
// so treat it the same as one that was never set at all.
pub fn optional(key: &str) -> Option<String> {
    env::var(key).ok().filter(|value| !value.trim().is_empty())
}
