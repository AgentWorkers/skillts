# Skill Translator Service

翻译服务，用于将 openclaw/skills 仓库中的英文 SKILL.md 文件翻译成中文版本。

## 功能特性

- 🌐 使用 OpenAI API 进行高质量翻译
- 💾 SQLite 缓存机制，避免重复翻译
- 🔄 增量更新，只翻译变更的文件
- 📝 智能处理 YAML Frontmatter 和代码块
- 🚀 Axum 高性能异步服务 (Rust)

## 项目结构

```
skillts/
├── src/
│   ├── main.rs               # 服务入口
│   ├── config.rs             # 配置管理
│   ├── error.rs              # 错误类型定义
│   ├── models/
│   │   └── schemas.rs        # 数据模型
│   ├── routers/
│   │   └── translate.rs      # 翻译 API 路由
│   └── services/
│       ├── translator.rs     # 翻译引擎
│       ├── cache.rs         # 缓存管理
│       └── parser.rs        # 内容解析器
├── data/
│   └── cache.db              # SQLite 缓存数据库
├── Cargo.toml
├── .env.example
└── README.md
```

## 快速开始

### 1. 安装 Rust

如果尚未安装 Rust，请访问 https://rustup.rs/ 安装。

### 2. 配置环境变量

```bash
cp .env.example .env
# 编辑 .env 文件，填入你的 OpenAI API Key
```

### 3. 启动翻译服务

```bash
# 开发模式运行（带热重载，需要安装 cargo-watch）
cargo run

# 或者直接运行
cargo run --release
```

## API 端点

### 翻译单个文件

```http
POST /api/translate
Authorization: Bearer <your-api-key>
Content-Type: application/json

{
    "content": "YmFzZTY0IGVuY29kZWQgY29udGVudA==",
    "path": "skills/owner/skill-name/SKILL.md",
    "content_hash": "sha256:abc123...",
    "options": {
        "source_language": "en",
        "target_language": "zh-CN"
    }
}
```

### 批量翻译

```http
POST /api/translate/batch
Authorization: Bearer <your-api-key>
Content-Type: application/json

{
    "files": [
        {
            "path": "skills/owner1/skill1/SKILL.md",
            "content": "YmFzZTY0...",
            "content_hash": "sha256:abc123..."
        }
    ],
    "skip_cached": true
}
```

### 健康检查

```http
GET /api/health
```

### 缓存统计

```http
GET /api/cache/stats
Authorization: Bearer <your-api-key>
```

### 清除缓存

```http
DELETE /api/cache?expired_only=true
Authorization: Bearer <your-api-key>
```

## 配置选项

| 环境变量 | 说明 | 默认值 |
|---------|------|--------|
| `OPENAI_API_KEY` | OpenAI API 密钥 | - |
| `OPENAI_MODEL` | 使用的模型 | `gpt-4o-mini` |
| `OPENAI_BASE_URL` | OpenAI API 基础 URL | `https://api.openai.com/v1` |
| `LOCAL_API_BEARER` | API 认证 Token | - |
| `HOST` | 服务监听地址 | `127.0.0.1` |
| `PORT` | 服务监听端口 | `8080` |
| `TRANSLATOR_VERSION` | 翻译器版本 | `1.0.0` |
| `TARGET_LANGUAGE` | 目标语言 | `zh-CN` |
| `SOURCE_LANGUAGE` | 源语言 | `en` |
| `MAX_CONCURRENT_TRANSLATIONS` | 最大并发翻译数 | `5` |
| `TRANSLATION_TIMEOUT_SECONDS` | 翻译超时时间（秒） | `600` |
| `MAX_TOKENS` | 最大 Token 数 | `16000` |
| `CACHE_DB_PATH` | 缓存数据库路径 | `./data/cache.db` |
| `CACHE_MAX_AGE_DAYS` | 缓存最大天数 | `30` |

## 翻译规则

### YAML Frontmatter 处理

- 保留原始格式
- 仅翻译 `description` 字段
- 保留 `name`, `version`, `author` 等技术字段不翻译

### 代码块处理

- 代码内容不翻译
- 代码注释可选择翻译（默认不翻译）
- 保留代码块的语言标识

### 专有名词

以下术语保留原文：
- OpenClaw
- ClawHub
- API
- CLI
- GitHub

### 行长度限制

- 超过 5000 字符的行会被静默丢弃

## 开发

### 构建命令

```bash
cargo build              # 构建项目
cargo run                # 运行服务
cargo test               # 运行测试
cargo clippy             # 代码检查
cargo fmt                # 代码格式化
```

### 运行测试

```bash
# 运行所有测试
cargo test

# 运行指定测试
cargo test test_name
```

## 许可证

MIT License
