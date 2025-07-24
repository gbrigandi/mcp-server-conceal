//! PII detection engine using regex pattern matching

use crate::config::{DetectedEntity, DetectionConfig};
use anyhow::Result;
use regex::Regex;
use serde_json::Value;
use std::collections::HashMap;
use tracing::{debug, warn};

#[derive(Clone)]
pub struct RegexDetectionEngine {
    patterns: HashMap<String, Regex>,
    confidence_threshold: f64,
}

impl RegexDetectionEngine {
    pub fn new(config: &DetectionConfig) -> Result<Self> {
        let mut patterns = HashMap::new();
        
        for (name, pattern_str) in &config.patterns {
            match Regex::new(pattern_str) {
                Ok(regex) => {
                    patterns.insert(name.clone(), regex);
                    debug!("Loaded regex pattern for '{}': {}", name, pattern_str);
                }
                Err(e) => {
                    warn!("Invalid regex pattern for '{}': {}", name, e);
                    return Err(anyhow::anyhow!("Invalid regex pattern for '{}': {}", name, e));
                }
            }
        }
        
        Ok(Self {
            patterns,
            confidence_threshold: config.confidence_threshold,
        })
    }

    pub fn detect_in_text(&self, text: &str) -> Vec<DetectedEntity> {
        let mut entities = Vec::new();
        
        for (entity_type, regex) in &self.patterns {
            for mat in regex.find_iter(text) {
                let entity = DetectedEntity {
                    entity_type: entity_type.clone(),
                    original_value: mat.as_str().to_string(),
                    start: mat.start(),
                    end: mat.end(),
                    confidence: self.calculate_confidence(entity_type, mat.as_str()),
                };
                
                if entity.confidence >= self.confidence_threshold {
                    entities.push(entity);
                }
            }
        }
        
        entities.sort_by_key(|e| e.start);
        entities
    }

    pub fn detect_in_json(&self, json: &Value) -> Vec<DetectedEntity> {
        let mut entities = Vec::new();
        self.traverse_json(json, &mut entities, String::new());
        entities
    }

    fn traverse_json(&self, value: &Value, entities: &mut Vec<DetectedEntity>, path: String) {
        match value {
            Value::String(s) => {
                let detected = self.detect_in_text(s);
                for mut entity in detected {
                    entity.entity_type = format!("{}@{}", entity.entity_type, path);
                    entities.push(entity);
                }
            }
            Value::Object(map) => {
                for (key, val) in map {
                    let new_path = if path.is_empty() {
                        key.clone()
                    } else {
                        format!("{}.{}", path, key)
                    };
                    self.traverse_json(val, entities, new_path);
                }
            }
            Value::Array(arr) => {
                for (index, val) in arr.iter().enumerate() {
                    let new_path = format!("{}[{}]", path, index);
                    self.traverse_json(val, entities, new_path);
                }
            }
            _ => {}
        }
    }

    // This is a simple heuristic. Good enough for now, but a proper
    // NLP model would be needed for higher accuracy.
    fn calculate_confidence(&self, entity_type: &str, text: &str) -> f64 {
        match entity_type {
            "email" => {
                if text.contains('@') && text.contains('.') {
                    0.95
                } else {
                    0.7
                }
            }
            "phone" => {
                let digit_count = text.chars().filter(|c| c.is_ascii_digit()).count();
                if digit_count >= 10 {
                    0.9
                } else {
                    0.6
                }
            }
            "ssn" => {
                if text.matches('-').count() == 2 {
                    0.95
                } else {
                    0.8
                }
            }
            "credit_card" => {
                let digit_count = text.chars().filter(|c| c.is_ascii_digit()).count();
                if digit_count == 16 {
                    0.85
                } else {
                    0.7
                }
            }
            "ip_address" => {
                let parts: Vec<&str> = text.split('.').collect();
                if parts.len() == 4 && parts.iter().all(|&p| p.parse::<u8>().is_ok()) {
                    0.95
                } else {
                    0.7
                }
            }
            "url" => {
                if text.starts_with("http://") || text.starts_with("https://") {
                    0.9
                } else {
                    0.7
                }
            }
            _ => 0.8,
        }
    }

    pub fn replace_entities_in_text(&self, text: &str, replacements: &HashMap<String, String>) -> String {
        let entities = self.detect_in_text(text);
        if entities.is_empty() {
            return text.to_string();
        }

        let mut result = String::new();
        let mut last_end = 0;

        for entity in entities {
            result.push_str(&text[last_end..entity.start]);
            if let Some(replacement) = replacements.get(&entity.original_value) {
                result.push_str(replacement);
                debug!("Replaced '{}' with '{}' at position {}-{}", 
                       entity.original_value, replacement, entity.start, entity.end);
            } else {
                result.push_str(&entity.original_value);
            }
            
            last_end = entity.end;
        }
        
        result.push_str(&text[last_end..]);
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{DetectionConfig, DetectionMode};
    use serde_json::json;
    use std::collections::HashMap;

    fn create_test_config() -> DetectionConfig {
        let mut patterns = HashMap::new();
        patterns.insert("email".to_string(), r"\b[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Z|a-z]{2,}\b".to_string());
        patterns.insert("phone".to_string(), r"\b\d{3}-\d{3}-\d{4}\b".to_string());
        patterns.insert("ssn".to_string(), r"\b\d{3}-\d{2}-\d{4}\b".to_string());
        patterns.insert("ip_address".to_string(), r"\b(?:(?:25[0-5]|2[0-4][0-9]|[01]?[0-9][0-9]?)\.){3}(?:25[0-5]|2[0-4][0-9]|[01]?[0-9][0-9]?)\b".to_string());
        
        DetectionConfig {
            mode: DetectionMode::Regex,
            enabled: true,
            patterns,
            confidence_threshold: 0.8,
        }
    }

    #[test]
    fn test_engine_creation() {
        let config = create_test_config();
        let engine = RegexDetectionEngine::new(&config).unwrap();
        
        assert_eq!(engine.patterns.len(), 4);
        assert_eq!(engine.confidence_threshold, 0.8);
    }

    #[test]
    fn test_email_detection() {
        let config = create_test_config();
        let engine = RegexDetectionEngine::new(&config).unwrap();
        
        let text = "Contact John at john.doe@example.com for more info";
        let entities = engine.detect_in_text(text);
        
        assert_eq!(entities.len(), 1);
        assert_eq!(entities[0].entity_type, "email");
        assert_eq!(entities[0].original_value, "john.doe@example.com");
        assert_eq!(entities[0].start, 16);
        assert_eq!(entities[0].end, 36);
        assert!(entities[0].confidence >= 0.8);
    }

    #[test]
    fn test_phone_detection() {
        let config = create_test_config();
        let engine = RegexDetectionEngine::new(&config).unwrap();
        
        let text = "Call me at 555-123-4567 today";
        let entities = engine.detect_in_text(text);
        
        assert_eq!(entities.len(), 1);
        assert_eq!(entities[0].entity_type, "phone");
        assert_eq!(entities[0].original_value, "555-123-4567");
    }

    #[test]
    fn test_ssn_detection() {
        let config = create_test_config();
        let engine = RegexDetectionEngine::new(&config).unwrap();
        
        let text = "SSN: 123-45-6789";
        let entities = engine.detect_in_text(text);
        
        assert_eq!(entities.len(), 1);
        assert_eq!(entities[0].entity_type, "ssn");
        assert_eq!(entities[0].original_value, "123-45-6789");
    }

    #[test]
    fn test_multiple_entities() {
        let config = create_test_config();
        let engine = RegexDetectionEngine::new(&config).unwrap();
        
        let text = "Email: john@test.com, Phone: 555-123-4567";
        let entities = engine.detect_in_text(text);
        
        assert_eq!(entities.len(), 2);
        assert_eq!(entities[0].entity_type, "email");
        assert_eq!(entities[1].entity_type, "phone");
    }

    #[test]
    fn test_json_detection() {
        let config = create_test_config();
        let engine = RegexDetectionEngine::new(&config).unwrap();
        
        let json_data = json!({
            "customer": {
                "name": "John Doe",
                "email": "john@example.com",
                "phone": "555-123-4567"
            },
            "metadata": {
                "items": ["test@email.com"]
            }
        });
        
        let entities = engine.detect_in_json(&json_data);
        
        assert_eq!(entities.len(), 3);
        assert!(entities.iter().any(|e| e.entity_type.contains("customer.email")));
        assert!(entities.iter().any(|e| e.entity_type.contains("customer.phone")));
        assert!(entities.iter().any(|e| e.entity_type.contains("metadata.items[0]")));
    }

    #[test]
    fn test_confidence_calculation() {
        let config = create_test_config();
        let engine = RegexDetectionEngine::new(&config).unwrap();
        
        assert!(engine.calculate_confidence("email", "test@example.com") > 0.9);
        assert!(engine.calculate_confidence("phone", "555-123-4567") > 0.8);
        assert!(engine.calculate_confidence("ssn", "123-45-6789") > 0.9);
    }

    #[test]
    fn test_confidence_threshold_filtering() {
        let mut config = create_test_config();
        config.confidence_threshold = 0.95;
        
        let engine = RegexDetectionEngine::new(&config).unwrap();
        let text = "Email: john@test.com";
        let entities = engine.detect_in_text(text);
        assert_eq!(entities.len(), 1);
    }

    #[test]
    fn test_text_replacement() {
        let config = create_test_config();
        let engine = RegexDetectionEngine::new(&config).unwrap();
        
        let text = "Contact john@example.com or call 555-123-4567";
        let mut replacements = HashMap::new();
        replacements.insert("john@example.com".to_string(), "fake@company.com".to_string());
        replacements.insert("555-123-4567".to_string(), "555-987-6543".to_string());
        
        let result = engine.replace_entities_in_text(text, &replacements);
        
        assert_eq!(result, "Contact fake@company.com or call 555-987-6543");
    }

    #[test]
    fn test_no_replacements() {
        let config = create_test_config();
        let engine = RegexDetectionEngine::new(&config).unwrap();
        
        let text = "No PII here, just plain text";
        let replacements = HashMap::new();
        
        let result = engine.replace_entities_in_text(text, &replacements);
        
        assert_eq!(result, text);
    }

    #[test]
    fn test_localhost_ip_detection() {
        let config = create_test_config();
        let engine = RegexDetectionEngine::new(&config).unwrap();
        
        let text = "Connect to 127.0.0.1:8080 and also 192.168.1.1";
        let detected = engine.detect_in_text(text);
        
        // Should find both IP addresses
        let ip_entities: Vec<_> = detected.iter().filter(|e| e.entity_type == "ip_address").collect();
        assert_eq!(ip_entities.len(), 2);
        
        // Check 127.0.0.1 detection
        let localhost_entity = ip_entities.iter().find(|e| e.original_value == "127.0.0.1").unwrap();
        assert_eq!(localhost_entity.original_value, "127.0.0.1");
        assert_eq!(localhost_entity.confidence, 0.95); // Should get high confidence
        
        // Check 192.168.1.1 detection
        let private_entity = ip_entities.iter().find(|e| e.original_value == "192.168.1.1").unwrap();
        assert_eq!(private_entity.original_value, "192.168.1.1");
        assert_eq!(private_entity.confidence, 0.95);
    }
}