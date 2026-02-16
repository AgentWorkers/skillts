//! Data models for API request and response schemas.
//!
//! Fully compatible with Python version's Pydantic models.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Options for translation
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct TranslateOptions {
    pub preserve_frontmatter: bool,
    pub preserve_code_blocks: bool,
    pub translate_code_comments: bool,
    pub target_language: String,
    pub source_language: String,
}

impl Default for TranslateOptions {
    fn default() -> Self {
        Self {
            preserve_frontmatter: true,
            preserve_code_blocks: true,
            translate_code_comments: false,
            target_language: "zh-CN".to_string(),
            source_language: "en".to_string(),
        }
    }
}

/// Request model for single file translation
#[derive(Debug, Deserialize)]
pub struct TranslateRequest {
    /// Base64 encoded content of the SKILL.md file
    pub content: String,
    /// Relative path of the file in the repository
    pub path: String,
    /// SHA256 hash of the original content (with "sha256:" prefix)
    pub content_hash: String,
    /// Optional translation options
    pub options: Option<TranslateOptions>,
}

/// Response model for single file translation
#[derive(Debug, Serialize)]
pub struct TranslateResponse {
    /// Base64 encoded translated content
    pub translated_content: String,
    /// SHA256 hash of the original content
    pub content_hash: String,
    /// SHA256 hash of the translated content
    pub translated_hash: String,
    /// Whether the result was retrieved from cache
    pub cached: bool,
    /// Additional metadata
    pub metadata: serde_json::Value,
}

/// Model for a single file in batch translation
#[derive(Debug, Deserialize)]
pub struct FileToTranslate {
    pub path: String,
    /// Base64 encoded content
    pub content: String,
    pub content_hash: String,
}

/// Request model for batch translation
#[derive(Debug, Deserialize)]
pub struct BatchTranslateRequest {
    pub files: Vec<FileToTranslate>,
    pub options: Option<TranslateOptions>,
    #[serde(default = "default_skip_cached")]
    pub skip_cached: bool,
}

fn default_skip_cached() -> bool {
    true
}

/// Result for a single file in batch translation
#[derive(Debug, Serialize)]
pub struct FileTranslationResult {
    pub path: String,
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub translated_content: Option<String>,
    pub content_hash: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub translated_hash: Option<String>,
    #[serde(default)]
    pub cached: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Response model for batch translation
#[derive(Debug, Serialize)]
pub struct BatchTranslateResponse {
    pub results: Vec<FileTranslationResult>,
    pub total_files: usize,
    pub successful: usize,
    pub cached_count: usize,
    pub failed: usize,
    pub processing_time_ms: f64,
}

/// Model for a cache entry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheEntry {
    pub cache_key: String,
    pub content_hash: String,
    pub path: String,
    pub translated_content: String,
    pub translated_hash: String,
    pub created_at: DateTime<Utc>,
    pub accessed_at: DateTime<Utc>,
    pub hit_count: i64,
    #[serde(default)]
    pub metadata: serde_json::Value,
}

/// Statistics about the cache
#[derive(Debug, Serialize)]
pub struct CacheStats {
    pub total_entries: i64,
    pub total_size_bytes: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub oldest_entry: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub newest_entry: Option<DateTime<Utc>>,
    pub total_hits: i64,
    pub total_misses: i64,
}

/// Health check response
#[derive(Debug, Serialize)]
pub struct HealthResponse {
    pub status: String,
    pub version: String,
    pub cache_connected: bool,
    pub openai_configured: bool,
}

/// Root endpoint response
#[derive(Debug, Serialize)]
pub struct RootResponse {
    pub service: String,
    pub version: String,
    pub description: String,
    pub endpoints: serde_json::Value,
}
