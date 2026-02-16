//! Skill Translator Service - Main entry point.
//!
//! A translation service for SKILL.md files using OpenAI API with caching support.
//! Written in Rust for better performance and lower memory usage.

mod config;
mod error;
mod models;
mod routers;
mod services;

use axum::{
    body::Body,
    http::{Request, Response},
    middleware::{self, Next},
    routing::{delete, get, post},
    Router,
};
use chrono::Timelike;
use std::sync::Arc;
use tokio::signal;
use tower_http::cors::{Any, CorsLayer};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use crate::config::get_settings;
use crate::routers::translate::{
    auth_middleware, clear_cache, clear_expired_cache, flush_cache_hits, get_cache_stats,
    health_check, root, translate_batch, translate_file, AppState,
};
use crate::services::cache::TranslationCache;
use crate::services::translator::Translator;

/// Access log middleware - FastAPI style
async fn access_log_middleware(
    req: Request<Body>,
    next: Next,
) -> Response<Body> {
    let start = std::time::Instant::now();
    
    // Get request info before moving req
    let method = req.method().clone();
    let uri = req.uri().clone();
    let version = req.version();
    
    // Get client IP from headers or connection info
    let client_ip = req
        .headers()
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("127.0.0.1")
        .split(',')
        .next()
        .unwrap_or("127.0.0.1")
        .trim()
        .to_string();
    
    // Process request
    let response = next.run(req).await;
    
    // Calculate duration
    let duration = start.elapsed();
    
    // Get status code
    let status = response.status();
    let status_code = status.as_u16();
    let status_text = if status.is_success() {
        "OK"
    } else if status.is_client_error() {
        "BAD REQUEST"
    } else if status.is_server_error() {
        "INTERNAL ERROR"
    } else {
        ""
    };
    
    // Format HTTP version
    let http_version = match version {
        axum::http::Version::HTTP_09 => "HTTP/0.9",
        axum::http::Version::HTTP_10 => "HTTP/1.0",
        axum::http::Version::HTTP_11 => "HTTP/1.1",
        axum::http::Version::HTTP_2 => "HTTP/2",
        axum::http::Version::HTTP_3 => "HTTP/3",
        _ => "HTTP/1.1",
    };
    
    // Log in FastAPI style with timestamp
    tracing::info!(
        r#"{}:{} - "{} {} {}" {} {}"#,
        client_ip,
        "-", // port not easily available
        method,
        uri.path(),
        http_version,
        status_code,
        status_text
    );
    
    tracing::debug!("Request completed in {:?}", duration);
    
    response
}

/// Backup cache database file before initialization
async fn backup_cache_db(db_path: &str) -> anyhow::Result<()> {
    use tokio::fs;

    let db_path = std::path::Path::new(db_path);

    // Only backup if the database file exists
    if !db_path.exists() {
        tracing::debug!("Cache database does not exist, skipping backup");
        return Ok(());
    }

    let backup_path = db_path.with_extension("bak.db");

    // Remove old backup if exists
    if backup_path.exists() {
        fs::remove_file(&backup_path).await?;
        tracing::debug!("Removed old backup: {:?}", backup_path);
    }

    // Copy current database to backup
    fs::copy(db_path, &backup_path).await?;
    tracing::info!("Cache database backed up to: {:?}", backup_path);

    Ok(())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize logging with timestamp
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "skill_translator=info".into()),
        )
        .with(
            tracing_subscriber::fmt::layer()
                .with_target(false)
                .with_timer(tracing_subscriber::fmt::time::time())
        )
        .init();

    // Load settings
    let settings = get_settings();

    tracing::info!(
        "Starting Skill Translator Service v{}",
        settings.translator_version
    );
    tracing::info!("OpenAI model: {}", settings.openai_model);
    tracing::info!("Cache database: {}", settings.cache_db_path);

    // Check OpenAI API key
    if settings.openai_api_key.is_empty() {
        tracing::warn!("OpenAI API key not configured. Translation will fail.");
    } else {
        tracing::info!("OpenAI API key configured");
    }

    // Check API bearer
    if settings.local_api_bearer.is_empty() {
        tracing::warn!("API bearer not configured. API will be open without authentication.");
    } else {
        tracing::info!("API authentication enabled");
    }

    // Backup cache database before initialization
    backup_cache_db(&settings.cache_db_path).await?;

    // Initialize cache
    let cache = Arc::new(TranslationCache::new().await?);
    tracing::info!("Cache initialized successfully");

    // Initialize translator
    let translator = Arc::new(Translator::new());

    // Get API bearer for authentication
    let api_bearer = settings.local_api_bearer.clone();

    // Clone cache for graceful shutdown (before moving into AppState)
    let cache_for_shutdown = cache.clone();

    // Clone cache for background cleanup task
    let cache_for_cleanup = cache.clone();

    // Start background cache cleanup task (runs daily at 1 AM)
    tokio::spawn(async move {
        loop {
            // Calculate time until next 1 AM
            let now = chrono::Local::now();
            let next_1am = now
                .with_hour(1)
                .and_then(|t| t.with_minute(0))
                .and_then(|t| t.with_second(0))
                .and_then(|t| t.with_nanosecond(0));

            let next_run = match next_1am {
                Some(t) if t > now => t,
                Some(t) => t + chrono::Duration::days(1), // Already passed today, schedule for tomorrow
                None => {
                    tracing::error!("Failed to calculate next cleanup time");
                    return;
                }
            };

            let sleep_duration = (next_run - now)
                .to_std()
                .unwrap_or(std::time::Duration::from_secs(3600));

            tracing::info!(
                "Cache cleanup scheduled for {} (in {} seconds)",
                next_run.format("%Y-%m-%d %H:%M:%S"),
                sleep_duration.as_secs()
            );

            tokio::time::sleep(sleep_duration).await;

            // Run cleanup: clear entries not accessed in 30 days
            match cache_for_cleanup.clear_stale(30).await {
                Ok(count) => {
                    tracing::info!("Daily cache cleanup completed: {} stale entries removed", count);
                }
                Err(e) => {
                    tracing::error!("Daily cache cleanup failed: {}", e);
                }
            }
        }
    });

    // Create application state
    let state = AppState {
        translator,
        cache,
        api_bearer,
    };

    // Health check route (no auth required)
    let health_route = Router::new().route("/api/health", get(health_check));

    // Build API routes with authentication
    let api_routes = Router::new()
        .route("/translate", post(translate_file))
        .route("/translate/batch", post(translate_batch))
        .route("/cache/stats", get(get_cache_stats))
        .route("/cache", delete(clear_cache))
        .route("/cache/expired", delete(clear_expired_cache))
        .route("/cache/flush", post(flush_cache_hits))
        .route_layer(axum::middleware::from_fn_with_state(
            state.clone(),
            auth_middleware,
        ))
        .with_state(state);

    // Build application
    let app = Router::new()
        .route("/", get(root))
        .merge(health_route)
        .nest("/api", api_routes)
        .layer(middleware::from_fn(access_log_middleware))
        .layer(
            CorsLayer::new()
                .allow_origin(Any)
                .allow_methods(Any)
                .allow_headers(Any),
        );

    // Build server address
    let addr = format!("{}:{}", settings.host, settings.port);
    let listener = tokio::net::TcpListener::bind(&addr).await?;

    tracing::info!("Server listening on {}", addr);

    // Setup graceful shutdown
    let shutdown_signal = async {
        let ctrl_c = async {
            signal::ctrl_c()
                .await
                .expect("Failed to install Ctrl+C handler");
        };

        #[cfg(unix)]
        let terminate = async {
            signal::unix::signal(signal::unix::SignalKind::terminate())
                .expect("Failed to install signal handler")
                .recv()
                .await;
        };

        #[cfg(not(unix))]
        let terminate = std::future::pending::<()>();

        tokio::select! {
            _ = ctrl_c => {},
            _ = terminate => {},
        }

        tracing::info!("Shutdown signal received, starting graceful shutdown...");
    };

    // Start server with graceful shutdown
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal)
        .await?;

    // Graceful shutdown: close cache connection
    if let Err(e) = cache_for_shutdown.close().await {
        tracing::error!("Error during cache shutdown: {}", e);
    }

    tracing::info!("Server shutdown complete");

    Ok(())
}
