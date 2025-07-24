//! Configuration management for mcp-server-conceal

use anyhow::Result;
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub detection: DetectionConfig,
    pub faker: FakerConfig,
    pub mapping: MappingConfig,
    pub llm: Option<LlmConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DetectionConfig {
    pub mode: DetectionMode,
    pub enabled: bool,
    pub patterns: HashMap<String, String>,
    pub confidence_threshold: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DetectionMode {
    Regex,
    Llm,
    #[serde(rename = "regex_llm")]
    RegexLlm,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FakerConfig {
    pub locale: String,
    pub seed: Option<u64>,
    pub consistency: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MappingConfig {
    pub database_path: PathBuf,
    pub encryption: bool,
    pub retention_days: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmConfig {
    pub enabled: bool,
    pub model: String,
    pub endpoint: String,
    pub timeout_seconds: u64,
    pub prompt_template: Option<String>,
}

impl Default for Config {
    fn default() -> Self {
        let mut patterns = HashMap::new();
        patterns.insert(
            "email".to_string(),
            r"\b[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Z|a-z]{2,}\b".to_string(),
        );
        // Add other common patterns here as needed
        
        Self {
            detection: DetectionConfig {
                mode: DetectionMode::RegexLlm,
                enabled: true,
                patterns,
                confidence_threshold: 0.8,
            },
            faker: FakerConfig {
                locale: "en_US".to_string(),
                seed: Some(12345),
                consistency: true,
            },
            mapping: MappingConfig {
                database_path: PathBuf::from("mappings.db"),
                encryption: false,
                retention_days: Some(90),
            },
            llm: Some(LlmConfig {
                enabled: true,
                model: "llama3.2:3b".to_string(),
                endpoint: "http://localhost:11434".to_string(),
                timeout_seconds: 300,
                prompt_template: None,
            }),
        }
    }
}

impl Config {
    pub fn get_app_dirs() -> Result<ProjectDirs> {
        ProjectDirs::from("com", "mcp-server-conceal", "mcp-server-conceal")
            .ok_or_else(|| anyhow::anyhow!("Failed to determine application directories"))
    }

    pub fn resolve_paths(&mut self) -> Result<()> {
        let project_dirs = Self::get_app_dirs()?;
        
        // Resolve database path if relative
        if self.mapping.database_path.is_relative() {
            let data_dir = project_dirs.data_dir();
            std::fs::create_dir_all(data_dir)?;
            self.mapping.database_path = data_dir.join(&self.mapping.database_path);
        }
        
        Ok(())
    }

    pub fn from_file<P: AsRef<std::path::Path>>(path: P) -> Result<Self> {
        let contents = std::fs::read_to_string(path)?;
        let mut config: Self = toml::from_str(&contents)?;
        config.resolve_paths()?;
        Ok(config)
    }

    pub fn get_default_config_path() -> Result<PathBuf> {
        let project_dirs = Self::get_app_dirs()?;
        let config_dir = project_dirs.config_dir();
        std::fs::create_dir_all(config_dir)?;
        Ok(config_dir.join("mcp-server-conceal.toml"))
    }

    pub fn to_file<P: AsRef<std::path::Path>>(&self, path: P) -> Result<()> {
        let contents = toml::to_string_pretty(self)?;
        std::fs::write(path, contents)?;
        Ok(())
    }

    pub fn validate(&self) -> Result<()> {
        for (name, pattern) in &self.detection.patterns {
            regex::Regex::new(pattern)
                .map_err(|e| anyhow::anyhow!("Invalid regex pattern for '{}': {}", name, e))?;
        }

        if !(0.0..=1.0).contains(&self.detection.confidence_threshold) {
            return Err(anyhow::anyhow!("Confidence threshold must be between 0.0 and 1.0"));
        }
        
        if let Some(parent) = self.mapping.database_path.parent() {
            if !parent.exists() {
                std::fs::create_dir_all(parent)?;
            }
        }

        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DetectedEntity {
    pub entity_type: String,
    pub original_value: String,
    pub start: usize,
    pub end: usize,
    pub confidence: f64,
}

#[derive(Debug, Clone)]
pub struct AnonymizedEntity {
    pub entity_type: String,
    pub original_value: String,
    pub fake_value: String,
    pub mapping_id: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    #[test]
    fn test_config_default() {
        let config = Config::default();
        
        assert!(config.detection.enabled);
        assert!(matches!(config.detection.mode, DetectionMode::RegexLlm));
        assert!(config.detection.patterns.contains_key("email"));
        assert_eq!(config.faker.locale, "en_US");
        assert_eq!(config.faker.seed, Some(12345));
        assert!(config.faker.consistency);
    }

    #[test]
    fn test_config_validation() {
        let mut config = Config::default();
        
        config.validate().unwrap();
        
        config.detection.patterns.insert("invalid".to_string(), "[".to_string());
        assert!(config.validate().is_err());
        
        config = Config::default();
        config.detection.confidence_threshold = 1.5;
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_config_file_operations() {
        let config = Config::default();
        let temp_file = NamedTempFile::new().unwrap();
        let temp_path = temp_file.path();
        
        config.to_file(temp_path).unwrap();
        
        let loaded_config = Config::from_file(temp_path).unwrap();
        assert_eq!(config.detection.enabled, loaded_config.detection.enabled);
        assert_eq!(config.faker.locale, loaded_config.faker.locale);
        assert_eq!(config.mapping.encryption, loaded_config.mapping.encryption);
    }

    #[test]
    fn test_detected_entity() {
        let entity = DetectedEntity {
            entity_type: "email".to_string(),
            original_value: "john@example.com".to_string(),
            start: 10,
            end: 25,
            confidence: 0.95,
        };
        
        assert_eq!(entity.entity_type, "email");
        assert_eq!(entity.original_value, "john@example.com");
        assert_eq!(entity.confidence, 0.95);
    }

    #[test]
    fn test_anonymized_entity() {
        let entity = AnonymizedEntity {
            entity_type: "email".to_string(),
            original_value: "john@example.com".to_string(),
            fake_value: "mike@testcorp.com".to_string(),
            mapping_id: "uuid-123".to_string(),
        };
        
        assert_eq!(entity.entity_type, "email");
        assert_eq!(entity.original_value, "john@example.com");
        assert_eq!(entity.fake_value, "mike@testcorp.com");
    }
}