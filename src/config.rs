//! Configuration management for skill-translator.
//!
//! Loads settings from environment variables and .env file.
//! Fully compatible with Python version's configuration format.

use std::env;
use std::path::PathBuf;
use std::sync::OnceLock;

/// Global settings instance
static SETTINGS: OnceLock<Settings> = OnceLock::new();

/// Application settings loaded from environment variables
#[derive(Debug, Clone)]
pub struct Settings {
    // OpenAI configuration
    pub openai_api_key: String,
    pub openai_model: String,
    pub openai_base_url: String,

    // Server configuration
    pub host: String,
    pub port: u16,
    #[allow(dead_code)]
    pub reload: bool,

    // API authentication
    pub local_api_bearer: String,

    // Translator configuration
    pub translator_version: String,
    pub target_language: String,
    pub source_language: String,

    // Performance configuration
    pub max_concurrent_translations: usize,
    pub translation_timeout_seconds: u64,
    pub max_tokens: u32,

    // Cache configuration
    pub cache_db_path: String,
    pub cache_max_age_days: i64,
}

impl Settings {
    /// Load settings from environment variables.
    /// First attempts to load .env file, then reads environment variables.
    pub fn load() -> Self {
        // Try to load .env file from current directory or parent directories
        if let Some(path) = find_env_file() {
            let _ = dotenvy::from_path(&path);
        }

        Settings {
            // OpenAI configuration
            openai_api_key: env::var("OPENAI_API_KEY").unwrap_or_default(),
            openai_model: env::var("OPENAI_MODEL").unwrap_or_else(|_| "gpt-4o-mini".to_string()),
            openai_base_url: env::var("OPENAI_BASE_URL")
                .unwrap_or_else(|_| "https://api.openai.com/v1".to_string()),

            // Server configuration
            host: env::var("HOST").unwrap_or_else(|_| "127.0.0.1".to_string()),
            port: env::var("PORT")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(8080),
            reload: env::var("RELOAD")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(false),

            // API authentication
            local_api_bearer: env::var("LOCAL_API_BEARER").unwrap_or_default(),

            // Translator configuration
            translator_version: env::var("TRANSLATOR_VERSION")
                .unwrap_or_else(|_| "1.0.0".to_string()),
            target_language: env::var("TARGET_LANGUAGE").unwrap_or_else(|_| "zh-CN".to_string()),
            source_language: env::var("SOURCE_LANGUAGE").unwrap_or_else(|_| "en".to_string()),

            // Performance configuration
            max_concurrent_translations: env::var("MAX_CONCURRENT_TRANSLATIONS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(5),
            translation_timeout_seconds: env::var("TRANSLATION_TIMEOUT_SECONDS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(600),
            max_tokens: env::var("MAX_TOKENS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(16000),

            // Cache configuration
            cache_db_path: env::var("CACHE_DB_PATH")
                .unwrap_or_else(|_| "./data/cache.db".to_string()),
            cache_max_age_days: env::var("CACHE_MAX_AGE_DAYS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(30),
        }
    }
}

/// Find .env file in current directory or parent directories
fn find_env_file() -> Option<PathBuf> {
    let current_dir = env::current_dir().ok()?;

    // Check current directory
    let env_path = current_dir.join(".env");
    if env_path.exists() {
        return Some(env_path);
    }

    // Check parent directories
    let mut parent = current_dir.parent();
    while let Some(dir) = parent {
        let env_path = dir.join(".env");
        if env_path.exists() {
            return Some(env_path);
        }
        parent = dir.parent();
    }

    None
}

/// Get the global settings instance.
/// Initializes settings on first call.
pub fn get_settings() -> &'static Settings {
    SETTINGS.get_or_init(Settings::load)
}
