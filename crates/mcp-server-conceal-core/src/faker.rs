//! Fake data generation for PII anonymization

use crate::config::{AnonymizedEntity, DetectedEntity, FakerConfig};
use anyhow::Result;
use fake::faker::internet::en::{SafeEmail, IP, DomainSuffix};
use fake::faker::name::en::{FirstName, LastName};
use fake::Fake;
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use std::collections::HashMap;
use tracing::{debug, warn};
use uuid::Uuid;

#[derive(Clone)]
pub struct FakerEngine {
    rng: StdRng,
    locale: String,
    consistency: bool,
}

impl FakerEngine {
    pub fn new(config: &FakerConfig) -> Self {
        let rng = if let Some(seed) = config.seed {
            StdRng::seed_from_u64(seed)
        } else {
            StdRng::from_entropy()
        };
        
        Self {
            rng,
            locale: config.locale.clone(),
            consistency: config.consistency,
        }
    }

    pub fn anonymize_entity(&mut self, detected: &DetectedEntity) -> Result<AnonymizedEntity> {
        let entity_type = self.extract_base_type(&detected.entity_type);
        
        let fake_value = match entity_type.as_str() {
            "email" => self.generate_fake_email(),
            "phone" => self.generate_fake_phone(),
            "ssn" => self.generate_fake_ssn(),
            "name" => self.generate_fake_name(),
            "ip_address" => self.generate_fake_ip(),
            "hostname" => self.generate_fake_hostname(),
            "node_name" => self.generate_fake_node_name(),
            _ => {
                warn!("Unknown entity type '{}', using generic replacement", entity_type);
                format!("REDACTED_{}", entity_type.to_uppercase())
            }
        };

        let mapping_id = Uuid::new_v4().to_string();
        
        debug!("Generated fake '{}' for entity type '{}': {} -> {}", 
               mapping_id, entity_type, detected.original_value, fake_value);

        Ok(AnonymizedEntity {
            entity_type: detected.entity_type.clone(),
            original_value: detected.original_value.clone(),
            fake_value,
            mapping_id,
        })
    }

    pub fn anonymize_entities(&mut self, detected_entities: Vec<DetectedEntity>) -> Result<Vec<AnonymizedEntity>> {
        detected_entities.into_iter()
            .map(|entity| self.anonymize_entity(&entity))
            .collect()
    }

    fn extract_base_type(&self, entity_type: &str) -> String {
        entity_type.split('@').next().unwrap_or(entity_type).to_string()
    }

    fn generate_fake_email(&mut self) -> String {
        SafeEmail().fake_with_rng(&mut self.rng)
    }

    fn generate_fake_phone(&mut self) -> String {
        // Just generate a simple fake phone number
        format!("555-{:03}-{:04}", 
            self.rng.gen_range(100..999), 
            self.rng.gen_range(1000..9999))
    }

    // Use 900s to ensure it's obviously fake
    fn generate_fake_ssn(&mut self) -> String {
        format!("9{:02}-{:02}-{:04}", 
            self.rng.gen_range(10..99),
            self.rng.gen_range(10..99),
            self.rng.gen_range(1000..9999))
    }

    fn generate_fake_name(&mut self) -> String {
        let first: String = FirstName().fake_with_rng(&mut self.rng);
        let last: String = LastName().fake_with_rng(&mut self.rng);
        format!("{} {}", first, last)
    }

    fn generate_fake_ip(&mut self) -> String {
        IP().fake_with_rng(&mut self.rng)
    }

    fn generate_fake_hostname(&mut self) -> String {
        // Generate a fake hostname like "server-04.example.com" or "web-proxy-01.local"
        let prefixes = ["server", "web", "db", "app", "proxy", "gateway", "host", "node"];
        let prefix = prefixes[self.rng.gen_range(0..prefixes.len())];
        let number = self.rng.gen_range(1..100);
        let domain_suffix: String = DomainSuffix().fake_with_rng(&mut self.rng);
        
        format!("{}-{:02}.fake.{}", prefix, number, domain_suffix)
    }

    fn generate_fake_node_name(&mut self) -> String {
        // Generate a fake node name like "node42", "worker-03", "master01"
        let node_types = ["node", "worker", "master", "compute", "edge"];
        let node_type = node_types[self.rng.gen_range(0..node_types.len())];
        let number = self.rng.gen_range(1..100);
        
        // Randomly choose format: node42, node-42, or node_42
        let format_choice = self.rng.gen_range(0..3);
        match format_choice {
            0 => format!("{}{:02}", node_type, number),      // node42
            1 => format!("{}-{:02}", node_type, number),     // node-42
            _ => format!("{}_{:02}", node_type, number),     // node_42
        }
    }

    pub fn create_replacement_map(&mut self, detected_entities: Vec<DetectedEntity>) -> Result<HashMap<String, String>> {
        let mut replacement_map = HashMap::new();
        
        for entity in detected_entities {
            let anonymized = self.anonymize_entity(&entity)?;
            replacement_map.insert(anonymized.original_value, anonymized.fake_value);
        }
        
        Ok(replacement_map)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{DetectedEntity, FakerConfig};

    fn create_test_config() -> FakerConfig {
        FakerConfig {
            locale: "en_US".to_string(),
            seed: Some(12345),
            consistency: true,
        }
    }

    #[test]
    fn test_faker_engine_creation() {
        let config = create_test_config();
        let engine = FakerEngine::new(&config);
        
        assert_eq!(engine.locale, "en_US");
        assert!(engine.consistency);
    }

    #[test]
    fn test_email_anonymization() {
        let config = create_test_config();
        let mut engine = FakerEngine::new(&config);
        
        let detected = DetectedEntity {
            entity_type: "email".to_string(),
            original_value: "john.doe@example.com".to_string(),
            start: 0,
            end: 20,
            confidence: 0.95,
        };
        
        let anonymized = engine.anonymize_entity(&detected).unwrap();
        
        assert_eq!(anonymized.entity_type, "email");
        assert_eq!(anonymized.original_value, "john.doe@example.com");
        assert!(anonymized.fake_value.contains('@'));
        assert_ne!(anonymized.fake_value, anonymized.original_value);
        assert!(!anonymized.mapping_id.is_empty());
    }

    #[test]
    fn test_phone_anonymization() {
        let config = create_test_config();
        let mut engine = FakerEngine::new(&config);
        
        let detected = DetectedEntity {
            entity_type: "phone".to_string(),
            original_value: "555-123-4567".to_string(),
            start: 0,
            end: 12,
            confidence: 0.9,
        };
        
        let anonymized = engine.anonymize_entity(&detected).unwrap();
        
        assert_eq!(anonymized.entity_type, "phone");
        assert!(anonymized.fake_value.contains('-'));
        assert_ne!(anonymized.fake_value, "555-123-4567");
    }

    #[test]
    fn test_ssn_anonymization() {
        let config = create_test_config();
        let mut engine = FakerEngine::new(&config);
        
        let detected = DetectedEntity {
            entity_type: "ssn".to_string(),
            original_value: "123-45-6789".to_string(),
            start: 0,
            end: 11,
            confidence: 0.95,
        };
        
        let anonymized = engine.anonymize_entity(&detected).unwrap();
        
        assert_eq!(anonymized.entity_type, "ssn");
        assert!(anonymized.fake_value.matches('-').count() == 2);
        assert_ne!(anonymized.fake_value, "123-45-6789");
        assert!(anonymized.fake_value.starts_with('9'));
    }

    #[test]
    fn test_consistency_with_seed() {
        let config = create_test_config();
        let mut engine1 = FakerEngine::new(&config);
        let mut engine2 = FakerEngine::new(&config);
        
        let detected = DetectedEntity {
            entity_type: "email".to_string(),
            original_value: "test@example.com".to_string(),
            start: 0,
            end: 16,
            confidence: 0.95,
        };
        
        let result1 = engine1.anonymize_entity(&detected).unwrap();
        let result2 = engine2.anonymize_entity(&detected).unwrap();
        
        assert_eq!(result1.fake_value, result2.fake_value);
    }

    #[test]
    fn test_multiple_entities_anonymization() {
        let config = create_test_config();
        let mut engine = FakerEngine::new(&config);
        
        let entities = vec![
            DetectedEntity {
                entity_type: "email".to_string(),
                original_value: "john@test.com".to_string(),
                start: 0, end: 13, confidence: 0.95,
            },
            DetectedEntity {
                entity_type: "phone".to_string(),
                original_value: "555-123-4567".to_string(),
                start: 20, end: 32, confidence: 0.9,
            },
        ];
        
        let anonymized = engine.anonymize_entities(entities).unwrap();
        
        assert_eq!(anonymized.len(), 2);
        assert!(anonymized[0].fake_value.contains('@'));
        assert!(anonymized[1].fake_value.contains('-'));
    }

    #[test]
    fn test_replacement_map_creation() {
        let config = create_test_config();
        let mut engine = FakerEngine::new(&config);
        
        let entities = vec![
            DetectedEntity {
                entity_type: "email".to_string(),
                original_value: "john@test.com".to_string(),
                start: 0, end: 13, confidence: 0.95,
            },
            DetectedEntity {
                entity_type: "phone".to_string(),
                original_value: "555-123-4567".to_string(),
                start: 20, end: 32, confidence: 0.9,
            },
        ];
        
        let replacement_map = engine.create_replacement_map(entities).unwrap();
        
        assert_eq!(replacement_map.len(), 2);
        assert!(replacement_map.contains_key("john@test.com"));
        assert!(replacement_map.contains_key("555-123-4567"));
        assert!(replacement_map["john@test.com"].contains('@'));
    }

    #[test]
    fn test_extract_base_type() {
        let config = create_test_config();
        let engine = FakerEngine::new(&config);
        
        assert_eq!(engine.extract_base_type("email"), "email");
        assert_eq!(engine.extract_base_type("email@customer.email"), "email");
        assert_eq!(engine.extract_base_type("phone@customer.phone"), "phone");
    }

    #[test]
    fn test_ip_address_anonymization() {
        let config = create_test_config();
        let mut engine = FakerEngine::new(&config);
        
        let detected = DetectedEntity {
            entity_type: "ip_address".to_string(),
            original_value: "10.0.0.1".to_string(),
            start: 0,
            end: 8,
            confidence: 0.95,
        };
        
        let anonymized = engine.anonymize_entity(&detected).unwrap();
        
        assert_eq!(anonymized.entity_type, "ip_address");
        assert_eq!(anonymized.original_value, "10.0.0.1");
        assert_ne!(anonymized.fake_value, "10.0.0.1");
        assert!(!anonymized.mapping_id.is_empty());
        
        // Verify it's a valid IP format (4 octets separated by dots)
        let parts: Vec<&str> = anonymized.fake_value.split('.').collect();
        assert_eq!(parts.len(), 4);
        
        // Verify each octet is a valid number (0-255)
        for part in parts {
            let _octet: u8 = part.parse().expect("Should be a valid number");
            // If parsing succeeds, it's automatically within 0-255 range for u8
        }
    }

    #[test]
    fn test_hostname_anonymization() {
        let config = create_test_config();
        let mut engine = FakerEngine::new(&config);
        
        let detected = DetectedEntity {
            entity_type: "hostname".to_string(),
            original_value: "ubuntu-linux-2404".to_string(),
            start: 0,
            end: 17,
            confidence: 0.95,
        };
        
        let anonymized = engine.anonymize_entity(&detected).unwrap();
        
        assert_eq!(anonymized.entity_type, "hostname");
        assert_eq!(anonymized.original_value, "ubuntu-linux-2404");
        assert_ne!(anonymized.fake_value, "ubuntu-linux-2404");
        assert!(!anonymized.mapping_id.is_empty());
        
        // Verify it's a valid hostname format
        assert!(anonymized.fake_value.contains(".fake."));
        assert!(anonymized.fake_value.contains("-"));
        
        // Verify it contains expected components
        let parts: Vec<&str> = anonymized.fake_value.split('.').collect();
        assert!(parts.len() >= 3); // at least prefix-number.fake.suffix
    }

    #[test]
    fn test_node_name_anonymization() {
        let config = create_test_config();
        let mut engine = FakerEngine::new(&config);
        
        let detected = DetectedEntity {
            entity_type: "node_name".to_string(),
            original_value: "node01".to_string(),
            start: 0,
            end: 6,
            confidence: 0.95,
        };
        
        let anonymized = engine.anonymize_entity(&detected).unwrap();
        
        assert_eq!(anonymized.entity_type, "node_name");
        assert_eq!(anonymized.original_value, "node01");
        assert_ne!(anonymized.fake_value, "node01");
        assert!(!anonymized.mapping_id.is_empty());
        
        // Verify it's a valid node name format
        assert!(anonymized.fake_value.contains("node") || 
                anonymized.fake_value.contains("worker") || 
                anonymized.fake_value.contains("master") ||
                anonymized.fake_value.contains("compute") ||
                anonymized.fake_value.contains("edge"));
    }

    #[test]
    fn test_unknown_entity_type() {
        let config = create_test_config();
        let mut engine = FakerEngine::new(&config);
        
        let detected = DetectedEntity {
            entity_type: "unknown_type".to_string(),
            original_value: "some_value".to_string(),
            start: 0, end: 10, confidence: 0.8,
        };
        
        let anonymized = engine.anonymize_entity(&detected).unwrap();
        
        assert_eq!(anonymized.fake_value, "REDACTED_UNKNOWN_TYPE");
    }

    #[test]
    fn test_localhost_ip_anonymization() {
        let config = create_test_config();
        let mut engine = FakerEngine::new(&config);
        
        let detected = DetectedEntity {
            entity_type: "ip_address".to_string(),
            original_value: "127.0.0.1".to_string(),
            start: 0,
            end: 9,
            confidence: 0.95,
        };
        
        let anonymized = engine.anonymize_entity(&detected).unwrap();
        
        assert_eq!(anonymized.entity_type, "ip_address");
        assert_eq!(anonymized.original_value, "127.0.0.1");
        assert_ne!(anonymized.fake_value, "127.0.0.1");
        assert!(!anonymized.mapping_id.is_empty());
        
        // Verify it's a valid IP format (4 octets separated by dots)
        let parts: Vec<&str> = anonymized.fake_value.split('.').collect();
        assert_eq!(parts.len(), 4);
        
        // Verify each octet is a valid number (0-255)
        for part in parts {
            let _octet: u8 = part.parse().expect("Should be a valid number");
        }
    }
}