//! Content parser for SKILL.md files.
//!
//! Handles YAML frontmatter and code block extraction/preservation.

use regex::Regex;
use serde_yaml_neo::Value as YamlValue;
use std::collections::HashMap;

/// Parsed SKILL.md content structure
#[derive(Debug, Clone)]
pub struct ParsedContent {
    /// Original frontmatter string (including --- delimiters)
    pub frontmatter: String,
    /// Parsed frontmatter as key-value map
    pub frontmatter_dict: HashMap<String, serde_json::Value>,
    /// Body content (after frontmatter)
    pub body: String,
    /// Extracted code blocks: (language, code, placeholder)
    pub code_blocks: Vec<(String, String, String)>,
}

/// Parser for SKILL.md files with special handling for frontmatter and code blocks
pub struct ContentParser {
    /// Pattern to match YAML frontmatter
    frontmatter_pattern: Regex,
    /// Pattern to match code blocks
    code_block_pattern: Regex,
}

impl ContentParser {
    /// Create a new content parser
    pub fn new() -> Self {
        Self {
            // (?s) enables DOTALL mode - makes . match newlines
            frontmatter_pattern: Regex::new(r"(?s)^---\s*\n(.*?)\n---\s*\n").unwrap(),
            code_block_pattern: Regex::new(r"(?s)```(\w*)\n(.*?)```").unwrap(),
        }
    }

    /// Parse SKILL.md content into structured components
    pub fn parse(&self, content: &str) -> ParsedContent {
        let mut frontmatter = String::new();
        let mut frontmatter_dict = HashMap::new();
        let mut body = content.to_string();

        // Extract frontmatter
        if let Some(caps) = self.frontmatter_pattern.captures(content) {
            frontmatter = caps.get(0).unwrap().as_str().to_string();
            let fm_content = caps.get(1).unwrap().as_str();
            frontmatter_dict = self.parse_yaml_frontmatter(fm_content);
            body = content[frontmatter.len()..].to_string();
        }

        // Extract code blocks
        let mut code_blocks = Vec::new();
        for (i, caps) in self.code_block_pattern.captures_iter(&body).enumerate() {
            let language = caps.get(1).map(|m| m.as_str().to_string()).unwrap_or_default();
            let code = caps.get(2).unwrap().as_str().to_string();
            let placeholder = format!("___CODE_BLOCK_{}___", i);
            code_blocks.push((language, code, placeholder));
        }

        ParsedContent {
            frontmatter,
            frontmatter_dict,
            body,
            code_blocks,
        }
    }

    /// Parse YAML frontmatter content into a dictionary using serde_yaml_neo
    fn parse_yaml_frontmatter(&self, fm_content: &str) -> HashMap<String, serde_json::Value> {
        match serde_yaml_neo::from_str::<YamlValue>(fm_content) {
            Ok(YamlValue::Mapping(map)) => {
                map.into_iter()
                    .filter_map(|(k, v)| {
                        k.as_str().map(|key| (key.to_string(), yaml_to_json_value(v)))
                    })
                    .collect()
            }
            _ => HashMap::new(),
        }
    }

    /// Replace code blocks with placeholders
    pub fn replace_code_blocks(&self, body: &str, code_blocks: &[(String, String, String)]) -> String {
        let mut result = body.to_string();

        for (language, code, placeholder) in code_blocks {
            let pattern = format!("```{}\n{}```", regex::escape(language), regex::escape(code));
            if let Ok(re) = Regex::new(&pattern) {
                result = re.replace(&result, placeholder.as_str()).to_string();
            }
        }

        result
    }

    /// Restore code blocks from placeholders
    pub fn restore_code_blocks(&self, body: &str, code_blocks: &[(String, String, String)]) -> String {
        let mut result = body.to_string();

        for (language, code, placeholder) in code_blocks {
            let restored = format!("```{}\n{}```", language, code);
            result = result.replace(placeholder, &restored);
        }

        result
    }

    /// Replace a specific field in the frontmatter with its translated value
    pub fn translate_frontmatter_field(
        &self,
        frontmatter: &str,
        field: &str,
        translated_value: &str,
    ) -> String {
        let lines: Vec<&str> = frontmatter.lines().collect();
        let mut result_lines = Vec::new();
        let mut i = 0;
        let field_prefix = format!("{}:", field);

        while i < lines.len() {
            let line = lines[i];

            // Check if this line starts with the target field
            if line.starts_with(&field_prefix) {
                let after_colon = &line[field_prefix.len()..].trim_start();

                // Check for block scalar indicators
                if *after_colon == ">" || *after_colon == "|" {
                    // Found a block scalar, need to replace entire block
                    
                    // Filter out empty lines from translated value to preserve YAML structure
                    let non_empty_lines: Vec<&str> = translated_value
                        .lines()
                        .filter(|line| !line.trim().is_empty())
                        .collect();
                    
                    // If only one non-empty line, use simple format
                    if non_empty_lines.len() == 1 {
                        result_lines.push(format!("{}: {}", field, non_empty_lines[0]));
                    } else {
                        // Multiple lines - use folded block format
                        result_lines.push(format!("{}: >", field));
                        // Add each non-empty line with proper indentation
                        for content_line in non_empty_lines {
                            result_lines.push(format!("  {}", content_line));
                        }
                    }

                    // Skip the block scalar indicator and all indented lines after it
                    i += 1;
                    while i < lines.len() {
                        let next_line = lines[i];
                        if next_line.trim().is_empty() {
                            // Empty line, still part of block
                            i += 1;
                            continue;
                        }
                        let indent = next_line.len() - next_line.trim_start().len();
                        if indent > 0 {
                            // Still in block content
                            i += 1;
                        } else {
                            // End of block
                            break;
                        }
                    }
                    continue;
                } else if after_colon.starts_with('"') && after_colon.ends_with('"') {
                    // Quoted string - preserve quotes
                    result_lines.push(format!("{}: \"{}\"", field, translated_value));
                } else if after_colon.starts_with('\'') && after_colon.ends_with('\'') {
                    // Single quoted string - preserve quotes
                    result_lines.push(format!("{}: '{}'", field, translated_value));
                } else if !after_colon.is_empty() {
                    // Regular unquoted value - check if translated value has newlines
                    if translated_value.contains('\n') {
                        // Filter out empty lines to preserve YAML structure
                        let non_empty_lines: Vec<&str> = translated_value
                            .lines()
                            .filter(|line| !line.trim().is_empty())
                            .collect();
                        
                        if non_empty_lines.len() == 1 {
                            result_lines.push(format!("{}: {}", field, non_empty_lines[0]));
                        } else {
                            // Need to use folded block format
                            result_lines.push(format!("{}: >", field));
                            for content_line in non_empty_lines {
                                result_lines.push(format!("  {}", content_line));
                            }
                        }
                    } else {
                        result_lines.push(format!("{}: {}", field, translated_value));
                    }
                } else {
                    // Empty value - just keep the field name
                    result_lines.push(line.to_string());
                }
            } else {
                result_lines.push(line.to_string());
            }

            i += 1;
        }

        result_lines.join("\n")
    }

    /// Get the description field from frontmatter
    pub fn get_description_field(&self, frontmatter_dict: &HashMap<String, serde_json::Value>) -> Option<String> {
        frontmatter_dict
            .get("description")
            .and_then(|v| v.as_str().map(|s| s.to_string()))
    }

    /// Check if a frontmatter field should be translated
    pub fn is_translatable_field(&self, field: &str) -> bool {
        matches!(field, "description")
    }
}

/// Convert YAML value to JSON value
fn yaml_to_json_value(v: YamlValue) -> serde_json::Value {
    match v {
        YamlValue::Null => serde_json::Value::Null,
        YamlValue::Bool(b) => serde_json::Value::Bool(b),
        YamlValue::Number(n) => {
            // Try to convert to serde_json::Number
            if let Some(n) = n.as_i64() {
                serde_json::Value::Number(n.into())
            } else if let Some(n) = n.as_f64() {
                serde_json::Number::from_f64(n)
                    .map(serde_json::Value::Number)
                    .unwrap_or(serde_json::Value::Null)
            } else {
                serde_json::Value::Null
            }
        }
        YamlValue::String(s) => serde_json::Value::String(s),
        YamlValue::Sequence(arr) => {
            serde_json::Value::Array(arr.into_iter().map(yaml_to_json_value).collect())
        }
        YamlValue::Mapping(map) => {
            serde_json::Value::Object(
                map.into_iter()
                    .filter_map(|(k, v)| {
                        k.as_str().map(|key| (key.to_string(), yaml_to_json_value(v)))
                    })
                    .collect(),
            )
        }
        // Tagged values (e.g., !!str) - extract the inner value
        YamlValue::Tagged(tagged) => yaml_to_json_value(tagged.value),
    }
}

impl Default for ContentParser {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_frontmatter() {
        let content = r#"---
name: test-skill
version: 1.0.0
description: "A test skill"
---

# Content here
"#;

        let parser = ContentParser::new();
        let parsed = parser.parse(content);

        assert!(!parsed.frontmatter.is_empty());
        assert_eq!(
            parsed.frontmatter_dict.get("name").and_then(|v| v.as_str()),
            Some("test-skill")
        );
        assert!(parsed.body.contains("# Content here"));
    }

    #[test]
    fn test_code_block_replacement() {
        let body = r#"Some text

```python
print("hello")
```

More text"#;

        let parser = ContentParser::new();
        let mut code_blocks = Vec::new();
        for (i, caps) in parser.code_block_pattern.captures_iter(body).enumerate() {
            let language = caps.get(1).map(|m| m.as_str().to_string()).unwrap_or_default();
            let code = caps.get(2).unwrap().as_str().to_string();
            let placeholder = format!("___CODE_BLOCK_{}___", i);
            code_blocks.push((language, code, placeholder));
        }

        let replaced = parser.replace_code_blocks(body, &code_blocks);
        assert!(replaced.contains("___CODE_BLOCK_0___"));
        assert!(!replaced.contains("print(\"hello\")"));

        let restored = parser.restore_code_blocks(&replaced, &code_blocks);
        assert!(restored.contains("print(\"hello\")"));
    }

    #[test]
    fn test_parse_frontmatter_with_multiline_metadata() {
        // Test case from real skill file with multi-line JSON metadata
        let content = r#"---
name: web-monitor
version: 3.1.0
description: "Monitor web pages for changes, price drops, stock availability, and custom conditions."
metadata:
  {
    "openclaw":
      {
        "emoji": "👁️",
        "requires": { "bins": ["python3", "curl"] },
      },
  }
---

# Web Monitor Pro
"#;

        let parser = ContentParser::new();
        let parsed = parser.parse(content);

        // Verify frontmatter was parsed
        assert!(!parsed.frontmatter.is_empty(), "Frontmatter should not be empty");

        // Verify description was correctly extracted
        let description = parser.get_description_field(&parsed.frontmatter_dict);
        assert!(description.is_some(), "Description should be extracted");
        assert!(
            description.unwrap().contains("Monitor web pages"),
            "Description should contain expected text"
        );

        // Verify name was parsed correctly
        assert_eq!(
            parsed.frontmatter_dict.get("name").and_then(|v| v.as_str()),
            Some("web-monitor")
        );

        // Verify body was separated correctly
        assert!(parsed.body.contains("# Web Monitor Pro"));
        assert!(!parsed.body.contains("description:"));
    }

    #[test]
    fn test_parse_frontmatter_inline_metadata() {
        // Test case with inline JSON metadata (no quotes around description)
        let content = r#"---
name: 0protocol
description: Agents can sign plugins, rotate credentials without losing identity.
homepage: https://github.com/0isone/0protocol
metadata: {"openclaw":{"emoji":"🪪","requires":{"bins":["mcporter"]}}}
---

# Content
"#;

        let parser = ContentParser::new();
        let parsed = parser.parse(content);

        // Verify frontmatter was parsed
        assert!(!parsed.frontmatter.is_empty(), "Frontmatter should not be empty");

        // Verify description was correctly extracted (unquoted)
        let description = parser.get_description_field(&parsed.frontmatter_dict);
        assert!(description.is_some(), "Description should be extracted");
        let desc = description.unwrap();
        assert!(
            desc.contains("Agents can sign plugins"),
            "Description should contain expected text: {:?}",
            desc
        );

        // Verify name was parsed correctly
        assert_eq!(
            parsed.frontmatter_dict.get("name").and_then(|v| v.as_str()),
            Some("0protocol")
        );
    }

    #[test]
    fn test_translate_frontmatter_field_quoted() {
        let frontmatter = r#"---
name: test
description: "This is a test description"
---
"#;
        let parser = ContentParser::new();
        let result = parser.translate_frontmatter_field(frontmatter, "description", "这是测试描述");

        assert!(result.contains(r#"description: "这是测试描述""#), "Should contain translated description with quotes: {}", result);
        assert!(!result.contains("This is a test description"));
    }

    #[test]
    fn test_translate_frontmatter_field_unquoted() {
        let frontmatter = r#"---
name: test
description: This is a test description without quotes
---
"#;
        let parser = ContentParser::new();
        let result = parser.translate_frontmatter_field(frontmatter, "description", "这是没有引号的测试描述");

        assert!(result.contains("description: 这是没有引号的测试描述"), "Should contain translated description without quotes: {}", result);
        assert!(!result.contains("This is a test description"));
    }

    #[test]
    fn test_parse_frontmatter_folded_description() {
        // Test case with YAML folded block scalar (>)
        let content = r#"---
name: solo-leveling
description: >
  Solo Leveling — a life RPG skill that turns real-world habits into an addictive progression
  system. Inspired by the manhwa Solo Leveling, this skill features 6 stats.
metadata:
  openclaw:
    emoji: "⚔️"
---

# Content
"#;

        let parser = ContentParser::new();
        let parsed = parser.parse(content);

        // Verify frontmatter was parsed
        assert!(!parsed.frontmatter.is_empty(), "Frontmatter should not be empty");

        // For folded block scalar, description value should be extracted
        // Note: YAML folded scalar parsing is complex, we need to handle multi-line content
        let description = parser.get_description_field(&parsed.frontmatter_dict);
        println!("Parsed description: {:?}", description);
        println!("Parsed frontmatter_dict: {:?}", parsed.frontmatter_dict);

        // The description field exists but may have special handling for folded scalars
        assert!(
            description.is_some() || parsed.frontmatter_dict.contains_key("description"),
            "Description field should exist"
        );
    }

    #[test]
    fn test_translate_frontmatter_field_folded() {
        // Test replacing a folded block scalar description
        let frontmatter = r#"---
name: test
description: >
  This is a multi-line description
  that spans multiple lines.
---
"#;
        let parser = ContentParser::new();
        let result = parser.translate_frontmatter_field(
            frontmatter,
            "description",
            "这是多行描述的翻译",
        );

        println!("Result: {}", result);
        // Should replace entire block with single line
        assert!(result.contains("description: 这是多行描述的翻译"), "Should contain translated description: {}", result);
        // Should NOT contain the original block content
        assert!(!result.contains("This is a multi-line"));
        assert!(!result.contains("that spans"));
        // Should NOT contain the > indicator
        assert!(!result.contains("description: >"));
    }

    #[test]
    fn test_translate_frontmatter_field_multiline_value() {
        // Test replacing with a multi-line translated value
        let frontmatter = r#"---
name: test
description: >
  This is a multi-line description
  that spans multiple lines.
---
"#;
        let parser = ContentParser::new();
        let translated = "这是第一行描述。\n这是第二行描述。\n这是第三行描述。";
        let result = parser.translate_frontmatter_field(
            frontmatter,
            "description",
            translated,
        );

        println!("Result:\n{}", result);
        // Should use folded block format for multiline value
        assert!(result.contains("description: >"), "Should use folded block format for multiline: {}", result);
        assert!(result.contains("  这是第一行描述。"), "Should have indented content: {}", result);
        assert!(result.contains("  这是第二行描述。"), "Should have indented content: {}", result);
        assert!(result.contains("  这是第三行描述。"), "Should have indented content: {}", result);
        // Should NOT contain the original content
        assert!(!result.contains("This is a multi-line"));
    }

    #[test]
    fn test_translate_frontmatter_field_with_empty_lines() {
        // Test replacing with a multi-line translated value that contains empty lines
        let frontmatter = r#"---
name: test
description: Some description here.
---
"#;
        let parser = ContentParser::new();
        // Translation with empty lines (common AI output format)
        let translated = "这是第一行描述。

这是第二行描述，前面有空行。

这是第三行描述。";
        let result = parser.translate_frontmatter_field(
            frontmatter,
            "description",
            translated,
        );

        println!("Result:\n{}", result);
        // Should use folded block format
        assert!(result.contains("description: >"), "Should use folded block format: {}", result);
        // Should have all non-empty lines with proper indentation
        assert!(result.contains("  这是第一行描述。"), "Should have first line: {}", result);
        assert!(result.contains("  这是第二行描述，前面有空行。"), "Should have second line: {}", result);
        assert!(result.contains("  这是第三行描述。"), "Should have third line: {}", result);
        // Should NOT have empty lines in the output (they would break YAML structure)
        // Count lines that are just whitespace - should be none after description field
        let lines: Vec<&str> = result.lines().collect();
        let desc_start = lines.iter().position(|l| l.starts_with("description:")).unwrap();
        let desc_lines: Vec<&&str> = lines[desc_start..].iter().take_while(|l| l.starts_with("  ") || l.starts_with("description:")).collect();
        for line in desc_lines {
            if line.starts_with("  ") {
                assert!(!line.trim().is_empty(), "Should not have empty lines in description block: {:?}", line);
            }
        }
    }
}
