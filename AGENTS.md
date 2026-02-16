# AGENTS.md

This file provides guidance to agents when working with code in this repository.

## Build & Run Commands

```bash
cargo build              # Build the project
cargo run                # Run the service (requires .env with OPENAI_API_KEY)
cargo test               # Run all tests (inline in .rs files)
cargo test <name>        # Run specific test by name
cargo clippy             # Lint with clippy
cargo fmt --check        # Check formatting
```

## Project-Specific Patterns

### Error Handling
- All handlers return `AppResult<T>` which is `Result<T, AppError>`
- `AppError` implements `IntoResponse` for Axum - do NOT wrap in `Json()` manually
- Use `?` operator for automatic error conversion (AppError has From implementations)

### State Management
- `AppState` in [`src/routers/translate.rs`](src/routers/translate.rs:30) uses `Arc<T>` for shared ownership
- Clone `Arc` fields before moving into background tasks (e.g., line 183-186 in main.rs)

### Content Encoding
- API accepts/produces base64-encoded content via [`encode_content()`](src/services/translator.rs) and [`decode_content()`](src/services/translator.rs)
- SHA256 hashes require "sha256:" prefix in cache keys

### Cache Behavior
- Cache auto-backs up to `.bak.db` on startup ([`backup_cache_db()`](src/main.rs:104))
- Background cleanup runs daily at 1 AM, removes entries not accessed in 30 days
- SQLite database requires `./data/` directory to exist

### Configuration
- Uses [`dotenvy`](src/config.rs) to load .env from current or parent directories
- Settings are global singleton via `get_settings()` - not reloaded at runtime

### Translation Rules
- Lines exceeding 5000 characters are silently dropped ([`MAX_LINE_LENGTH`](src/routers/translate.rs:26))
- YAML Frontmatter preserved, only translates `description` field
- Code blocks preserved, comments not translated by default
