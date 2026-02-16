//! API routes for translation service.
//!
//! Fully compatible with Python version's API endpoints.

use axum::{
    extract::State,
    http::{header, Request, StatusCode},
    middleware::Next,
    response::Response,
    Json,
};
use serde_json::json;
use std::sync::Arc;
use std::time::Instant;

use crate::config::get_settings;
use crate::error::AppError;
use crate::models::schemas::{
    BatchTranslateRequest, BatchTranslateResponse, CacheStats, FileTranslationResult,
    HealthResponse, RootResponse, TranslateRequest, TranslateResponse,
};
use crate::services::cache::TranslationCache;
use crate::services::translator::{decode_content, encode_content, Translator};

/// Maximum line length before filtering
const MAX_LINE_LENGTH: usize = 5000;

/// Application state shared across handlers
#[derive(Clone)]
pub struct AppState {
    pub translator: Arc<Translator>,
    pub cache: Arc<TranslationCache>,
    pub api_bearer: String,
}

/// Auth middleware for API endpoints
pub async fn auth_middleware(
    State(state): State<AppState>,
    request: Request<axum::body::Body>,
    next: Next,
) -> Result<Response, (StatusCode, Json<serde_json::Value>)> {
    // Skip auth if no bearer is configured
    if state.api_bearer.is_empty() {
        return Ok(next.run(request).await);
    }

    // Extract Authorization header
    let auth_header = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|h| h.to_str().ok());

    match auth_header {
        Some(header_value) => {
            // Check if it starts with "Bearer "
            if let Some(token) = header_value.strip_prefix("Bearer ") {
                if token == state.api_bearer {
                    Ok(next.run(request).await)
                } else {
                    Err((
                        StatusCode::UNAUTHORIZED,
                        Json(json!({ "detail": "Invalid API key" })),
                    ))
                }
            } else {
                Err((
                    StatusCode::UNAUTHORIZED,
                    Json(json!({ "detail": "Invalid Authorization header format" })),
                ))
            }
        }
        None => Err((
            StatusCode::UNAUTHORIZED,
            Json(json!({ "detail": "Missing Authorization header" })),
        )),
    }
}

/// Filter lines exceeding MAX_LINE_LENGTH
fn filter_long_lines(content: &str) -> (String, usize) {
    let lines: Vec<&str> = content.lines().collect();
    let mut filtered = Vec::new();
    let mut removed = 0;

    for line in lines {
        if line.len() <= MAX_LINE_LENGTH {
            filtered.push(line);
        } else {
            removed += 1;
        }
    }

    if removed > 0 {
        (filtered.join("\n"), removed)
    } else {
        (content.to_string(), 0)
    }
}

/// Root endpoint with service information
pub async fn root() -> Json<RootResponse> {
    let settings = get_settings();
    Json(RootResponse {
        service: "Skill Translator".to_string(),
        version: settings.translator_version.clone(),
        description: "Translation service for SKILL.md files".to_string(),
        endpoints: json!({
            "translate": "/api/translate",
            "batch": "/api/translate/batch",
            "health": "/api/health",
            "cache_stats": "/api/cache/stats"
        }),
    })
}

/// Health check endpoint (no auth required)
pub async fn health_check() -> Json<HealthResponse> {
    let settings = get_settings();
    Json(HealthResponse {
        status: "healthy".to_string(),
        version: settings.translator_version.clone(),
        cache_connected: true,
        openai_configured: !settings.openai_api_key.is_empty(),
    })
}

/// Translate a single SKILL.md file
#[axum::debug_handler]
pub async fn translate_file(
    State(state): State<AppState>,
    Json(request): Json<TranslateRequest>,
) -> Result<Json<TranslateResponse>, AppError> {
    let start_time = Instant::now();

    // Decode content
    let content = decode_content(&request.content)?;

    // Filter out lines exceeding 5000 characters
    let (content, removed_count) = filter_long_lines(&content);
    if removed_count > 0 {
        tracing::info!(
            "Removed {} lines exceeding {} characters",
            removed_count,
            MAX_LINE_LENGTH
        );
    }

    // Get options
    let settings = get_settings();
    let source_language = request
        .options
        .as_ref()
        .map(|o| o.source_language.as_str())
        .unwrap_or_else(|| settings.source_language.as_str());
    let target_language = request
        .options
        .as_ref()
        .map(|o| o.target_language.as_str())
        .unwrap_or_else(|| settings.target_language.as_str());

    // Compute cache key
    let cache_key = state
        .translator
        .compute_cache_key(&request.content_hash, source_language, target_language);

    // Check cache
    if let Some(cached) = state.cache.get(&cache_key).await? {
        let encoded_cached_content = encode_content(&cached.translated_content);
        return Ok(Json(TranslateResponse {
            translated_content: encoded_cached_content,
            content_hash: cached.content_hash,
            translated_hash: cached.translated_hash,
            cached: true,
            metadata: cached.metadata,
        }));
    }

    // Translate
    let (translated_content, metadata) = state
        .translator
        .translate(&content, source_language, target_language)
        .await?;

    // Compute hash of translated content
    let translated_hash = Translator::compute_hash(&translated_content);

    // Store in cache
    state.cache.set(
        &cache_key,
        &request.content_hash,
        &request.path,
        &translated_content,
        &translated_hash,
        Some(json!({
            "original_chars": metadata.original_chars,
            "translated_chars": metadata.translated_chars,
            "processing_time_ms": metadata.processing_time_ms,
            "translator_version": metadata.translator_version,
            "model": metadata.model,
            "source_language": metadata.source_language,
            "target_language": metadata.target_language,
        })),
    ).await?;

    // Encode response
    let encoded_content = encode_content(&translated_content);

    let processing_time = start_time.elapsed().as_millis() as f64;

    Ok(Json(TranslateResponse {
        translated_content: encoded_content,
        content_hash: request.content_hash,
        translated_hash,
        cached: false,
        metadata: json!({
            "original_chars": metadata.original_chars,
            "translated_chars": metadata.translated_chars,
            "processing_time_ms": metadata.processing_time_ms,
            "translator_version": metadata.translator_version,
            "model": metadata.model,
            "source_language": metadata.source_language,
            "target_language": metadata.target_language,
            "total_processing_time_ms": processing_time,
        }),
    }))
}

/// Translate multiple SKILL.md files in batch
#[axum::debug_handler]
pub async fn translate_batch(
    State(state): State<AppState>,
    Json(request): Json<BatchTranslateRequest>,
) -> Result<Json<BatchTranslateResponse>, AppError> {
    let start_time = Instant::now();

    let settings = get_settings();
    let source_language = request
        .options
        .as_ref()
        .map(|o| o.source_language.as_str())
        .unwrap_or_else(|| settings.source_language.as_str());
    let target_language = request
        .options
        .as_ref()
        .map(|o| o.target_language.as_str())
        .unwrap_or_else(|| settings.target_language.as_str());

    let mut results = Vec::new();
    let mut successful = 0usize;
    let mut cached_count = 0usize;
    let mut failed = 0usize;

    for file in request.files {
        match process_single_file(
            &state,
            &file.content,
            &file.content_hash,
            &file.path,
            source_language,
            target_language,
            request.skip_cached,
        )
        .await
        {
            Ok(result) => {
                if result.cached {
                    cached_count += 1;
                }
                if result.success {
                    successful += 1;
                } else {
                    failed += 1;
                }
                results.push(result);
            }
            Err(e) => {
                failed += 1;
                results.push(FileTranslationResult {
                    path: file.path,
                    success: false,
                    translated_content: None,
                    content_hash: file.content_hash,
                    translated_hash: None,
                    cached: false,
                    error: Some(e.to_string()),
                });
            }
        }
    }

    let processing_time = start_time.elapsed().as_millis() as f64;

    Ok(Json(BatchTranslateResponse {
        results,
        total_files: successful + failed,
        successful,
        cached_count,
        failed,
        processing_time_ms: processing_time,
    }))
}

/// Process a single file for batch translation
async fn process_single_file(
    state: &AppState,
    content_encoded: &str,
    content_hash: &str,
    path: &str,
    source_language: &str,
    target_language: &str,
    skip_cached: bool,
) -> Result<FileTranslationResult, AppError> {
    // Decode content
    let content = decode_content(content_encoded)?;

    // Filter out lines exceeding 5000 characters
    let (content, removed_count) = filter_long_lines(&content);
    if removed_count > 0 {
        tracing::info!(
            "[{}] Removed {} lines exceeding {} characters",
            path,
            removed_count,
            MAX_LINE_LENGTH
        );
    }

    // Compute cache key
    let cache_key = state
        .translator
        .compute_cache_key(content_hash, source_language, target_language);

    // Check cache
    if skip_cached {
        if let Some(cached) = state.cache.get(&cache_key).await? {
            let encoded_cached = encode_content(&cached.translated_content);
            return Ok(FileTranslationResult {
                path: path.to_string(),
                success: true,
                translated_content: Some(encoded_cached),
                content_hash: cached.content_hash,
                translated_hash: Some(cached.translated_hash),
                cached: true,
                error: None,
            });
        }
    }

    // Translate
    let (translated_content, _metadata) = state
        .translator
        .translate(&content, source_language, target_language)
        .await?;

    // Compute hash
    let translated_hash = Translator::compute_hash(&translated_content);

    // Store in cache
    state.cache.set(
        &cache_key,
        content_hash,
        path,
        &translated_content,
        &translated_hash,
        None,
    ).await?;

    // Encode response
    let encoded_content = encode_content(&translated_content);

    Ok(FileTranslationResult {
        path: path.to_string(),
        success: true,
        translated_content: Some(encoded_content),
        content_hash: content_hash.to_string(),
        translated_hash: Some(translated_hash),
        cached: false,
        error: None,
    })
}

/// Get cache statistics
pub async fn get_cache_stats(
    State(state): State<AppState>,
) -> Result<Json<CacheStats>, AppError> {
    let stats = state.cache.get_stats().await?;
    Ok(Json(stats))
}

/// Clear cache endpoint
pub async fn clear_cache(
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, AppError> {
    let cleared = state.cache.clear_all().await?;
    Ok(Json(json!({
        "message": format!("Cleared all {} entries", cleared)
    })))
}

/// Clear expired cache entries
pub async fn clear_expired_cache(
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, AppError> {
    let cleared = state.cache.clear_expired().await?;
    Ok(Json(json!({
        "message": format!("Cleared {} expired entries", cleared)
    })))
}

/// Flush pending hit count updates
pub async fn flush_cache_hits(
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, AppError> {
    state.cache.flush_pending_hits().await?;
    Ok(Json(json!({
        "message": "Flushed pending hits"
    })))
}