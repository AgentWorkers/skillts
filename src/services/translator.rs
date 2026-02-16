//! Translation engine using OpenAI API.
//!
//! Supports streaming responses, concurrent translation control, and retry logic.

use async_openai::{
    types::{
        ChatCompletionRequestMessage, ChatCompletionRequestSystemMessageArgs,
        ChatCompletionRequestUserMessageArgs, CreateChatCompletionRequestArgs,
    },
    Client, config::OpenAIConfig,
};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use futures::StreamExt;
use sha2::{Digest, Sha256};
use std::time::{Duration, Instant};
use tokio::sync::Semaphore;
use tokio::time::timeout;

use crate::config::get_settings;
use crate::error::{AppError, AppResult, TranslationError};
use crate::services::parser::ContentParser;

/// System prompt for translation
const SYSTEM_PROMPT: &str = r#"You are a professional technical translator specializing in software documentation.
Your task is to translate SKILL.md files from English to Chinese (Simplified, zh-CN).

IMPORTANT RULES:
1. Translate the content naturally while preserving technical accuracy
2. Keep all code examples, commands, and URLs unchanged
3. Preserve the markdown formatting exactly
4. Keep technical terms in English when appropriate (e.g., OpenClaw, ClawHub, API, CLI)
5. Translate comments in code blocks only if they are clearly explanatory
6. Maintain the same structure and organization as the original
7. Do not add or remove any sections
8. Preserve all placeholders like ___CODE_BLOCK_0___ exactly as they are

Translate the following content to Chinese (Simplified):"#;

/// Translation engine for SKILL.md files using OpenAI API
pub struct Translator {
    client: Client<OpenAIConfig>,
    model: String,
    max_tokens: u32,
    parser: ContentParser,
    translator_version: String,
    semaphore: Semaphore,
    timeout_seconds: u64,
    max_retries: u32,
    retry_delay: Duration,
}

/// Metadata for translation result
#[derive(Debug, Clone)]
pub struct TranslationMetadata {
    pub original_chars: usize,
    pub translated_chars: usize,
    pub processing_time_ms: f64,
    pub translator_version: String,
    pub model: String,
    pub source_language: String,
    pub target_language: String,
}

impl Translator {
    /// Create a new translator instance
    pub fn new() -> Self {
        let settings = get_settings();

        // Configure OpenAI client
        let config = OpenAIConfig::new()
            .with_api_key(&settings.openai_api_key)
            .with_api_base(&settings.openai_base_url);

        let client = Client::with_config(config);

        Self {
            client,
            model: settings.openai_model.clone(),
            max_tokens: settings.max_tokens,
            parser: ContentParser::new(),
            translator_version: settings.translator_version.clone(),
            semaphore: Semaphore::new(settings.max_concurrent_translations),
            timeout_seconds: settings.translation_timeout_seconds,
            max_retries: 3,
            retry_delay: Duration::from_secs(2),
        }
    }

    /// Compute SHA256 hash of content with prefix
    pub fn compute_hash(content: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(content.as_bytes());
        let hash = hasher.finalize();
        format!("sha256:{}", hex::encode(hash))
    }

    /// Compute cache key from content hash and translation parameters
    pub fn compute_cache_key(
        &self,
        content_hash: &str,
        source_language: &str,
        target_language: &str,
    ) -> String {
        let key_data = format!(
            "{}:{}:{}:{}",
            content_hash, source_language, target_language, self.translator_version
        );
        Self::compute_hash(&key_data)
    }

    /// Translate SKILL.md content from source to target language
    pub async fn translate(
        &self,
        content: &str,
        source_language: &str,
        target_language: &str,
    ) -> AppResult<(String, TranslationMetadata)> {
        let start_time = Instant::now();

        // Parse the content
        let parsed = self.parser.parse(content);

        // Replace code blocks with placeholders
        let body_with_placeholders = self
            .parser
            .replace_code_blocks(&parsed.body, &parsed.code_blocks);

        // Translate the body with concurrency control
        let translated_body = self
            .translate_with_control(&body_with_placeholders, source_language, target_language)
            .await?;

        // Restore code blocks
        let translated_body = self
            .parser
            .restore_code_blocks(&translated_body, &parsed.code_blocks);

        // Translate frontmatter description if present
        let translated_frontmatter = if let Some(description) =
            self.parser.get_description_field(&parsed.frontmatter_dict)
        {
            if !description.is_empty() && self.parser.is_translatable_field("description") {
                let translated_description = self
                    .translate_with_control(&description, source_language, target_language)
                    .await?;
                
                // Filter out empty lines to preserve YAML structure
                let cleaned_description: String = translated_description
                    .lines()
                    .filter(|line| !line.trim().is_empty())
                    .collect::<Vec<_>>()
                    .join("\n");
                
                self.parser.translate_frontmatter_field(
                    &parsed.frontmatter,
                    "description",
                    &cleaned_description,
                )
            } else {
                parsed.frontmatter.clone()
            }
        } else {
            parsed.frontmatter.clone()
        };

        // Combine frontmatter and translated body
        let translated_content = translated_frontmatter + &translated_body;

        // Compute metadata
        let processing_time = start_time.elapsed();
        let metadata = TranslationMetadata {
            original_chars: content.len(),
            translated_chars: translated_content.len(),
            processing_time_ms: processing_time.as_millis() as f64,
            translator_version: self.translator_version.clone(),
            model: self.model.clone(),
            source_language: source_language.to_string(),
            target_language: target_language.to_string(),
        };

        Ok((translated_content, metadata))
    }

    /// Translate text with concurrency control and timeout
    async fn translate_with_control(
        &self,
        text: &str,
        _source_language: &str,
        _target_language: &str,
    ) -> AppResult<String> {
        if text.trim().is_empty() {
            return Ok(text.to_string());
        }

        let _permit = self.semaphore.acquire().await.map_err(|_| {
            AppError::Internal("Failed to acquire semaphore permit".to_string())
        })?;

        let result = timeout(
            Duration::from_secs(self.timeout_seconds),
            self.translate_text(text),
        )
        .await
        .map_err(|_| TranslationError::Timeout(self.timeout_seconds))??;

        Ok(result)
    }

    /// Translate text using OpenAI API with retry logic
    async fn translate_text(&self, text: &str) -> AppResult<String> {
        if text.trim().is_empty() {
            return Ok(text.to_string());
        }

        let mut last_error: Option<String> = None;

        for attempt in 0..self.max_retries {
            // Only wait before retry (not on first attempt)
            if attempt > 0 {
                tokio::time::sleep(self.retry_delay * attempt as u32).await;
            }

            match self.call_openai_api(text).await {
                Ok(content) => {
                    if !content.is_empty() {
                        return Ok(content);
                    }
                    return Err(TranslationError::EmptyResponse.into());
                }
                Err(e) => {
                    last_error = Some(e.to_string());
                }
            }
        }

        Err(TranslationError::RetryFailed {
            attempts: self.max_retries,
            error: last_error.unwrap_or_else(|| "Unknown error".to_string()),
        }
        .into())
    }

    /// Call OpenAI API with streaming
    async fn call_openai_api(&self, text: &str) -> AppResult<String> {
        let request = CreateChatCompletionRequestArgs::default()
            .model(&self.model)
            .messages(vec![
                ChatCompletionRequestMessage::System(
                    ChatCompletionRequestSystemMessageArgs::default()
                        .content(SYSTEM_PROMPT)
                        .build()?,
                ),
                ChatCompletionRequestMessage::User(
                    ChatCompletionRequestUserMessageArgs::default()
                        .content(text)
                        .build()?,
                ),
            ])
            .temperature(0.3)
            .max_tokens(self.max_tokens)
            .stream(true)
            .build()?;

        let mut stream = self.client.chat().create_stream(request).await?;

        let mut content_chunks = Vec::new();

        while let Some(response) = stream.next().await {
            match response {
                Ok(chunk) => {
                    for choice in chunk.choices {
                        if let Some(content) = choice.delta.content {
                            content_chunks.push(content);
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!("Stream error: {}", e);
                    return Err(TranslationError::OpenAIError(e.to_string()).into());
                }
            }
        }

        let content = content_chunks.join("");
        Ok(content.trim().to_string())
    }
}

impl Default for Translator {
    fn default() -> Self {
        Self::new()
    }
}

/// Encode content to base64 for API transmission
pub fn encode_content(content: &str) -> String {
    BASE64.encode(content.as_bytes())
}

/// Decode content from base64
pub fn decode_content(encoded: &str) -> AppResult<String> {
    let bytes = BASE64.decode(encoded.as_bytes())?;
    String::from_utf8(bytes).map_err(|e| AppError::BadRequest(format!("Invalid UTF-8 content: {}", e)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compute_hash() {
        let content = "hello world";
        let hash = Translator::compute_hash(content);
        assert!(hash.starts_with("sha256:"));
        assert_eq!(hash.len(), 71); // "sha256:" + 64 hex chars
    }

    #[test]
    fn test_encode_decode_content() {
        let original = "Hello, 世界!";
        let encoded = encode_content(original);
        let decoded = decode_content(&encoded).unwrap();
        assert_eq!(original, decoded);
    }
}