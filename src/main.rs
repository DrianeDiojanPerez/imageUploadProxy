mod auth;
mod config;
mod handlers;
mod logging;
mod repository;
mod usage;

use std::sync::Arc;

use axum::extract::{DefaultBodyLimit, Request};
use axum::routing::{delete, get, post};
use axum::Router;
use tokio::net::TcpListener;
use tower_http::trace::{DefaultOnFailure, DefaultOnResponse, TraceLayer};
use tracing::Level;

use crate::config::Config;
use crate::handlers::AppState;
use crate::repository::github::GitHubRepository;
use crate::repository::ImageRepository;

// Multipart framing, the filename, and the part headers all ride along with
// the file, so the body limit sits above the file limit and lets the handler
// be the one that answers a slightly oversized upload.
const BODY_LIMIT_SLACK: usize = 1024 * 1024;

#[tokio::main]
async fn main() {
    // A .env file is only a convenience for local runs, so a real exported
    // variable always wins and a missing file is not an error.
    dotenvy::dotenv().ok();

    logging::init();

    let config = Config::from_env();

    // The concrete backend is chosen here and nowhere else, so swapping it
    // later is a change to this line rather than to any handler.
    let repository: Arc<dyn ImageRepository + Send + Sync> =
        Arc::new(GitHubRepository::new(config.github).expect("could not build the github repository"));

    let body_limit = config.settings.max_upload_bytes + BODY_LIMIT_SLACK;

    if config.settings.upload_secret.is_none() {
        tracing::warn!("UPLOAD_SECRET is not set, so uploads and deletes are open to anyone");
    }

    tracing::info!(
        max_upload_mb = config.settings.max_upload_mb(),
        types = %config.settings.allowed_list(),
        "accepting uploads"
    );

    let state = AppState {
        repository,
        settings: Arc::new(config.settings),
    };

    // The span carries only the method and the path. Headers stay out of it
    // so an upload secret never reaches the logs, and the body hook is off
    // so the image itself is never touched by the logger.
    let trace = TraceLayer::new_for_http()
        .make_span_with(|request: &Request| {
            tracing::info_span!(
                "request",
                method = %request.method(),
                path = %request.uri().path(),
            )
        })
        .on_body_chunk(())
        .on_response(DefaultOnResponse::new().level(Level::INFO))
        .on_failure(DefaultOnFailure::new().level(Level::ERROR));

    // The secret guards writing only. Reads have to stay open or an image
    // would never render in a browser or an img tag, and the uuid in the
    // path is what keeps a stored image from being found by guessing.
    let guard = axum::middleware::from_fn_with_state(state.clone(), auth::require_secret);

    let app = Router::new()
        .route("/upload", post(handlers::upload).route_layer(guard.clone()))
        // Behind the guard too. It says nothing about the images, but it does
        // say how much of the account's budget is left, which is nobody
        // else's business.
        .route("/usage", get(handlers::usage).route_layer(guard.clone()))
        .route(
            "/images/:name",
            get(handlers::serve).merge(delete(handlers::remove).route_layer(guard)),
        )
        .layer(DefaultBodyLimit::max(body_limit))
        .layer(trace)
        .with_state(state);

    let listener = TcpListener::bind(&config.bind_address)
        .await
        .unwrap_or_else(|e| panic!("could not bind to {}: {e}", config.bind_address));

    tracing::info!(address = %config.bind_address, "listening");

    axum::serve(listener, app)
        .await
        .expect("the server stopped unexpectedly");
}

