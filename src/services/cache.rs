//! Cache management for translations using SQLite.
//!
//! Fully compatible with Python version's cache implementation.
//! Uses WAL mode for better concurrent performance.

use chrono::{DateTime, Duration, Utc};
use sqlx::sqlite::{SqlitePool, SqlitePoolOptions};
use sqlx::Row;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::config::get_settings;
use crate::error::{AppError, AppResult};
use crate::models::schemas::{CacheEntry, CacheStats};

/// SQLite-based cache for translations with performance optimizations
pub struct TranslationCache {
    pool: SqlitePool,
    max_age_days: i64,
    miss_count: Arc<Mutex<i64>>,
    pending_hits: Arc<Mutex<HashMap<String, i64>>>,
}

impl TranslationCache {
    /// Create a new cache instance
    pub async fn new() -> AppResult<Self> {
        let settings = get_settings();
        let db_path = &settings.cache_db_path;

        // Ensure parent directory exists
        let path = Path::new(db_path);
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await.map_err(|e| {
                AppError::Internal(format!("Failed to create cache directory: {}", e))
            })?;
        }

        // Build SQLite connection URL
        let db_url = format!("sqlite:{}?mode=rwc", db_path);

        // Create connection pool (reduced for low-memory VPS)
        let pool = SqlitePoolOptions::new()
            .max_connections(2)
            .connect(&db_url)
            .await?;

        // Enable WAL mode and other optimizations
        Self::enable_wal_mode(&pool).await?;

        // Initialize schema
        Self::init_schema(&pool).await?;

        Ok(Self {
            pool,
            max_age_days: settings.cache_max_age_days,
            miss_count: Arc::new(Mutex::new(0)),
            pending_hits: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    /// Enable WAL mode for better concurrent performance
    async fn enable_wal_mode(pool: &SqlitePool) -> AppResult<()> {
        sqlx::query("PRAGMA journal_mode=WAL")
            .execute(pool)
            .await?;
        sqlx::query("PRAGMA synchronous=NORMAL")
            .execute(pool)
            .await?;
        // Reduced cache size for low-memory VPS (8MB instead of 64MB)
        sqlx::query("PRAGMA cache_size=-8000")
            .execute(pool)
            .await?;
        sqlx::query("PRAGMA busy_timeout=5000") // 5s timeout
            .execute(pool)
            .await?;
        // Auto-checkpoint every 100 pages to prevent WAL file from growing too large
        sqlx::query("PRAGMA wal_autocheckpoint=100")
            .execute(pool)
            .await?;
        Ok(())
    }

    /// Initialize the database schema with optimized indexes
    async fn init_schema(pool: &SqlitePool) -> AppResult<()> {
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS translations (
                cache_key TEXT PRIMARY KEY,
                content_hash TEXT NOT NULL,
                path TEXT NOT NULL,
                translated_content TEXT NOT NULL,
                translated_hash TEXT NOT NULL,
                created_at TEXT NOT NULL,
                accessed_at TEXT NOT NULL,
                hit_count INTEGER DEFAULT 0,
                metadata TEXT DEFAULT '{}'
            )
            "#,
        )
        .execute(pool)
        .await?;

        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_content_hash ON translations(content_hash)",
        )
        .execute(pool)
        .await?;

        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_path ON translations(path)",
        )
        .execute(pool)
        .await?;

        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_created_at ON translations(created_at)",
        )
        .execute(pool)
        .await?;

        Ok(())
    }

    /// Get a cached translation
    pub async fn get(&self, cache_key: &str) -> AppResult<Option<CacheEntry>> {
        let row = sqlx::query(
            "SELECT * FROM translations WHERE cache_key = ?",
        )
        .bind(cache_key)
        .fetch_optional(&self.pool)
        .await?;

        match row {
            Some(row) => {
                let created_at_str: String = row.get("created_at");
                let created_at = DateTime::parse_from_rfc3339(&created_at_str)
                    .map(|dt| dt.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now());

                // Check expiration
                let now = Utc::now();
                if now - created_at > Duration::days(self.max_age_days) {
                    // Delete expired entry
                    sqlx::query("DELETE FROM translations WHERE cache_key = ?")
                        .bind(cache_key)
                        .execute(&self.pool)
                        .await?;

                    let mut miss_count = self.miss_count.lock().await;
                    *miss_count += 1;
                    return Ok(None);
                }

                // Queue hit count update
                let mut pending = self.pending_hits.lock().await;
                *pending.entry(cache_key.to_string()).or_insert(0) += 1;

                let accessed_at_str: String = row.get("accessed_at");
                let accessed_at = DateTime::parse_from_rfc3339(&accessed_at_str)
                    .map(|dt| dt.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now());

                let hit_count: i64 = row.get("hit_count");
                let pending_hit = pending.get(cache_key).copied().unwrap_or(0);

                let metadata_str: String = row.get("metadata");
                let metadata = serde_json::from_str(&metadata_str).unwrap_or(serde_json::json!({}));

                Ok(Some(CacheEntry {
                    cache_key: row.get("cache_key"),
                    content_hash: row.get("content_hash"),
                    path: row.get("path"),
                    translated_content: row.get("translated_content"),
                    translated_hash: row.get("translated_hash"),
                    created_at,
                    accessed_at,
                    hit_count: hit_count + pending_hit - 1,
                    metadata,
                }))
            }
            None => {
                let mut miss_count = self.miss_count.lock().await;
                *miss_count += 1;
                Ok(None)
            }
        }
    }

    /// Store a translation in the cache
    pub async fn set(
        &self,
        cache_key: &str,
        content_hash: &str,
        path: &str,
        translated_content: &str,
        translated_hash: &str,
        metadata: Option<serde_json::Value>,
    ) -> AppResult<CacheEntry> {
        let now = Utc::now();
        let now_str = now.to_rfc3339();
        let metadata_clone = metadata.clone();
        let metadata_json = serde_json::to_string(&metadata.unwrap_or(serde_json::json!({})))
            .unwrap_or_else(|_| "{}".to_string());

        sqlx::query(
            r#"
            INSERT OR REPLACE INTO translations
            (cache_key, content_hash, path, translated_content, translated_hash,
             created_at, accessed_at, hit_count, metadata)
            VALUES (?, ?, ?, ?, ?, ?, ?, 0, ?)
            "#,
        )
        .bind(cache_key)
        .bind(content_hash)
        .bind(path)
        .bind(translated_content)
        .bind(translated_hash)
        .bind(&now_str)
        .bind(&now_str)
        .bind(&metadata_json)
        .execute(&self.pool)
        .await?;

        Ok(CacheEntry {
            cache_key: cache_key.to_string(),
            content_hash: content_hash.to_string(),
            path: path.to_string(),
            translated_content: translated_content.to_string(),
            translated_hash: translated_hash.to_string(),
            created_at: now,
            accessed_at: now,
            hit_count: 0,
            metadata: metadata_clone.unwrap_or(serde_json::json!({})),
        })
    }

    /// Flush pending hit count updates to database
    pub async fn flush_pending_hits(&self) -> AppResult<()> {
        let pending = {
            let mut pending = self.pending_hits.lock().await;
            std::mem::take(&mut *pending)
        };

        if pending.is_empty() {
            return Ok(());
        }

        let now = Utc::now().to_rfc3339();

        for (cache_key, count) in pending {
            sqlx::query(
                "UPDATE translations SET accessed_at = ?, hit_count = hit_count + ? WHERE cache_key = ?",
            )
            .bind(&now)
            .bind(count)
            .bind(&cache_key)
            .execute(&self.pool)
            .await?;
        }

        Ok(())
    }

    /// Clear all expired cache entries
    pub async fn clear_expired(&self) -> AppResult<i64> {
        let cutoff = (Utc::now() - Duration::days(self.max_age_days)).to_rfc3339();

        let result = sqlx::query("DELETE FROM translations WHERE created_at < ?")
            .bind(&cutoff)
            .execute(&self.pool)
            .await?;

        Ok(result.rows_affected() as i64)
    }

    /// Clear stale cache entries not accessed for specified days
    /// This is useful for cleaning up entries that haven't been used
    pub async fn clear_stale(&self, stale_days: i64) -> AppResult<i64> {
        let cutoff = (Utc::now() - Duration::days(stale_days)).to_rfc3339();

        let result = sqlx::query("DELETE FROM translations WHERE accessed_at < ?")
            .bind(&cutoff)
            .execute(&self.pool)
            .await?;

        tracing::info!(
            "Cleared {} stale cache entries (not accessed in {} days)",
            result.rows_affected(),
            stale_days
        );

        Ok(result.rows_affected() as i64)
    }

    /// Clear all cache entries
    pub async fn clear_all(&self) -> AppResult<i64> {
        let result = sqlx::query("DELETE FROM translations")
            .execute(&self.pool)
            .await?;

        Ok(result.rows_affected() as i64)
    }

    /// Get cache statistics
    pub async fn get_stats(&self) -> AppResult<CacheStats> {
        // Total entries
        let total_row = sqlx::query("SELECT COUNT(*) as count FROM translations")
            .fetch_one(&self.pool)
            .await?;
        let total_entries: i64 = total_row.get("count");

        // Total size
        let size_row = sqlx::query("SELECT SUM(LENGTH(translated_content)) as size FROM translations")
            .fetch_one(&self.pool)
            .await?;
        let total_size_bytes: i64 = size_row.get::<Option<i64>, _>("size").unwrap_or(0);

        // Oldest and newest entries
        let dates_row = sqlx::query("SELECT MIN(created_at) as oldest, MAX(created_at) as newest FROM translations")
            .fetch_one(&self.pool)
            .await?;

        let oldest: Option<String> = dates_row.get("oldest");
        let newest: Option<String> = dates_row.get("newest");

        let oldest_entry = oldest
            .and_then(|s| DateTime::parse_from_rfc3339(&s).ok())
            .map(|dt| dt.with_timezone(&Utc));

        let newest_entry = newest
            .and_then(|s| DateTime::parse_from_rfc3339(&s).ok())
            .map(|dt| dt.with_timezone(&Utc));

        // Total hits
        let hits_row = sqlx::query("SELECT SUM(hit_count) as hits FROM translations")
            .fetch_one(&self.pool)
            .await?;
        let total_hits: i64 = hits_row.get::<Option<i64>, _>("hits").unwrap_or(0);

        let miss_count = *self.miss_count.lock().await;

        Ok(CacheStats {
            total_entries,
            total_size_bytes,
            oldest_entry,
            newest_entry,
            total_hits,
            total_misses: miss_count,
        })
    }

    /// Gracefully close the cache connection
    /// Flushes pending hits and checkpoints WAL file
    pub async fn close(&self) -> AppResult<()> {
        tracing::info!("Closing cache connection...");

        // Flush any pending hit count updates
        self.flush_pending_hits().await?;
        tracing::debug!("Flushed pending hits");

        // Checkpoint WAL file to main database
        sqlx::query("PRAGMA wal_checkpoint(TRUNCATE)")
            .execute(&self.pool)
            .await?;
        tracing::debug!("Checkpointed WAL file");

        tracing::info!("Cache closed successfully");
        Ok(())
    }
}
