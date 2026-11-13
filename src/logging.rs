use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{fmt, EnvFilter};

use crate::config::optional;

/// Anything noisier than this comes from the dependencies, so the default
/// filter names only the crates we actually want to hear from.
const DEFAULT_FILTER: &str = "image_upload_proxy=info,tower_http=info";

/// Start the subscriber, picking the shape of the logs from LOG_FORMAT.
///
/// This runs before the config is read so a boot failure still comes out
/// through the same subscriber as everything else.
pub fn init() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(DEFAULT_FILTER));
    let registry = tracing_subscriber::registry().with(filter);

    let json = optional("LOG_FORMAT")
        .map(|format| format.eq_ignore_ascii_case("json"))
        .unwrap_or(false);

    // Flattening the event puts the message and its fields at the top level
    // of each line, which is the shape most log shippers expect to parse.
    if json {
        registry
            .with(
                fmt::layer()
                    .json()
                    .flatten_event(true)
                    .with_current_span(true)
                    .with_span_list(false),
            )
            .init();
    } else {
        registry.with(fmt::layer().pretty()).init();
    }
}
