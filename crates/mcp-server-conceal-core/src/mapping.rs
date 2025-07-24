//! Entity mapping storage using SQLite
//!
//! This module provides persistent storage for PII entity mappings and LLM cache entries,
//! ensuring consistency across sessions and supporting batch operations for performance.

use crate::config::{AnonymizedEntity, DetectedEntity, MappingConfig};
use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension};
use std::collections::HashMap;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};
use tracing::{debug, info, warn};
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct EntityMapping {
    pub id: String,
    pub entity_type: String,
    pub original_value_hash: String,
    pub fake_value: String,
    pub created_at: u64,
}

#[derive(Debug, Clone)]
pub struct LlmCacheEntry {
    pub id: String,
    pub text_hash: String,
    pub original_text: String,
    pub llm_result: Vec<DetectedEntity>,
    pub model_name: String,
    pub created_at: u64,
}

pub struct MappingStore {
    conn: Connection,
    config: MappingConfig,
}

impl MappingStore {
    pub fn new(config: MappingConfig) -> Result<Self> {
        let conn = if config.database_path == Path::new(":memory:") {
            Connection::open_in_memory()?
        } else {
            if let Some(parent) = config.database_path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            Connection::open(&config.database_path)?
        };

        let mut store = Self { conn, config };
        store.initialize_schema()?;
        store.cleanup_expired_mappings()?;
        
        info!("Initialized mapping store at {:?}", store.config.database_path);
        Ok(store)
    }

    fn initialize_schema(&mut self) -> Result<()> {
        self.conn.execute(
            "CREATE TABLE IF NOT EXISTS entity_mappings (
                id TEXT PRIMARY KEY,
                entity_type TEXT NOT NULL,
                original_value_hash TEXT NOT NULL,
                fake_value TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                UNIQUE(entity_type, original_value_hash)
            )",
            [],
        )?;

        self.conn.execute(
            "CREATE TABLE IF NOT EXISTS llm_cache (
                id TEXT PRIMARY KEY,
                text_hash TEXT NOT NULL,
                original_text TEXT NOT NULL,
                llm_result TEXT NOT NULL,
                model_name TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                UNIQUE(text_hash, model_name)
            )",
            [],
        )?;

        self.conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_entity_lookup 
             ON entity_mappings(entity_type, original_value_hash)",
            [],
        )?;

        self.conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_created_at 
             ON entity_mappings(created_at)",
            [],
        )?;

        self.conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_llm_cache_lookup 
             ON llm_cache(text_hash, model_name)",
            [],
        )?;

        self.conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_llm_cache_created_at 
             ON llm_cache(created_at)",
            [],
        )?;

        debug!("Database schema initialized");
        Ok(())
    }

    pub fn store_mapping(&mut self, anonymized: &AnonymizedEntity) -> Result<()> {
        let original_hash = self.hash_value(&anonymized.original_value);
        let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();

        self.conn.execute(
            "INSERT OR IGNORE INTO entity_mappings 
             (id, entity_type, original_value_hash, fake_value, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                anonymized.mapping_id,
                anonymized.entity_type,
                original_hash,
                anonymized.fake_value,
                now
            ],
        )?;

        debug!("Stored mapping for entity type '{}': {} -> {}", 
               anonymized.entity_type, original_hash, anonymized.fake_value);
        Ok(())
    }

    pub fn get_mapping(&self, entity_type: &str, original_value: &str) -> Result<Option<String>> {
        let original_hash = self.hash_value(original_value);
        
        let fake_value: Option<String> = self.conn
            .query_row(
                "SELECT fake_value FROM entity_mappings 
                 WHERE entity_type = ?1 AND original_value_hash = ?2",
                params![entity_type, original_hash],
                |row| row.get(0),
            )
            .optional()?;

        if let Some(ref value) = fake_value {
            debug!("Retrieved mapping for '{}': {} -> {}", 
                   entity_type, original_hash, value);
        }

        Ok(fake_value)
    }

    pub fn store_mappings_batch(&mut self, anonymized_entities: &[AnonymizedEntity]) -> Result<()> {
        let hashed_entities: Vec<_> = anonymized_entities.iter()
            .map(|e| (e, self.hash_value(&e.original_value)))
            .collect();
        
        let tx = self.conn.transaction()?;
        let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();

        {
            let mut stmt = tx.prepare(
                "INSERT OR IGNORE INTO entity_mappings 
                 (id, entity_type, original_value_hash, fake_value, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)"
            )?;

            for (anonymized, original_hash) in hashed_entities {
                stmt.execute(params![
                    anonymized.mapping_id,
                    anonymized.entity_type,
                    original_hash,
                    anonymized.fake_value,
                    now
                ])?;
            }
        }

        tx.commit()?;
        debug!("Stored batch of {} mappings", anonymized_entities.len());
        Ok(())
    }

    pub fn get_mappings_batch(&self, requests: &[(String, String)]) -> Result<HashMap<String, String>> {
        let mut results = HashMap::new();
        
        let mut stmt = self.conn.prepare(
            "SELECT fake_value FROM entity_mappings 
             WHERE entity_type = ?1 AND original_value_hash = ?2"
        )?;

        for (entity_type, original_value) in requests {
            let original_hash = self.hash_value(original_value);
            
            if let Some(fake_value) = stmt
                .query_row(params![entity_type, original_hash], |row| {
                    row.get::<_, String>(0)
                })
                .optional()?
            {
                results.insert(original_value.clone(), fake_value);
            }
        }

        debug!("Retrieved batch of {} mappings from {} requests", 
               results.len(), requests.len());
        Ok(results)
    }

    pub fn cleanup_expired_mappings(&mut self) -> Result<usize> {
        if let Some(retention_days) = self.config.retention_days {
            let cutoff_time = SystemTime::now()
                .duration_since(UNIX_EPOCH)?
                .as_secs()
                .saturating_sub(retention_days as u64 * 24 * 60 * 60);

            let deleted_mappings = self.conn.execute(
                "DELETE FROM entity_mappings WHERE created_at < ?1",
                params![cutoff_time],
            )?;

            let deleted_cache = self.conn.execute(
                "DELETE FROM llm_cache WHERE created_at < ?1",
                params![cutoff_time],
            )?;

            let total_deleted = deleted_mappings + deleted_cache;
            if total_deleted > 0 {
                info!("Cleaned up {} expired entries ({} mappings, {} cache) older than {} days", 
                      total_deleted, deleted_mappings, deleted_cache, retention_days);
            }

            Ok(total_deleted)
        } else {
            Ok(0)
        }
    }

    pub fn store_llm_cache(&mut self, text: &str, entities: &[DetectedEntity], model_name: &str) -> Result<()> {
        let text_hash = self.hash_value(text);
        let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
        let id = Uuid::new_v4().to_string();
        let llm_result_json = serde_json::to_string(entities)?;

        self.conn.execute(
            "INSERT OR REPLACE INTO llm_cache 
             (id, text_hash, original_text, llm_result, model_name, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![id, text_hash, text, llm_result_json, model_name, now],
        )?;

        debug!("Stored LLM cache entry for text hash '{}' with {} entities", 
               text_hash, entities.len());
        Ok(())
    }

    pub fn get_llm_cache(&self, text: &str, model_name: &str) -> Result<Option<Vec<DetectedEntity>>> {
        let text_hash = self.hash_value(text);
        
        let cache_result: Option<String> = self.conn
            .query_row(
                "SELECT llm_result FROM llm_cache 
                 WHERE text_hash = ?1 AND model_name = ?2",
                params![text_hash, model_name],
                |row| row.get(0),
            )
            .optional()?;

        if let Some(llm_result_json) = cache_result {
            let entities: Vec<DetectedEntity> = serde_json::from_str(&llm_result_json)?;
            debug!("Retrieved LLM cache hit for text hash '{}': {} entities", 
                   text_hash, entities.len());
            Ok(Some(entities))
        } else {
            debug!("LLM cache miss for text hash '{}' with model '{}'", text_hash, model_name);
            Ok(None)
        }
    }

    pub fn clear_llm_cache(&mut self) -> Result<usize> {
        let deleted = self.conn.execute("DELETE FROM llm_cache", [])?;
        warn!("Cleared all {} LLM cache entries from database", deleted);
        Ok(deleted)
    }

    pub fn get_statistics(&self) -> Result<MappingStatistics> {
        let total_mappings: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM entity_mappings",
            [],
            |row| row.get(0),
        )?;

        let total_cache_entries: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM llm_cache",
            [],
            |row| row.get(0),
        )?;

        let mut type_counts = HashMap::new();
        let mut stmt = self.conn.prepare(
            "SELECT entity_type, COUNT(*) FROM entity_mappings GROUP BY entity_type"
        )?;

        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })?;

        for row in rows {
            let (entity_type, count) = row?;
            type_counts.insert(entity_type, count as usize);
        }

        let oldest_mapping: Option<u64> = self.conn
            .query_row(
                "SELECT MIN(created_at) FROM entity_mappings WHERE created_at IS NOT NULL",
                [],
                |row| row.get::<_, Option<u64>>(0),
            )
            .optional()?
            .flatten();

        Ok(MappingStatistics {
            total_mappings: total_mappings as usize,
            total_cache_entries: total_cache_entries as usize,
            mappings_by_type: type_counts,
            oldest_mapping_age: oldest_mapping,
        })
    }

    pub fn clear_all_mappings(&mut self) -> Result<usize> {
        let deleted = self.conn.execute("DELETE FROM entity_mappings", [])?;
        warn!("Cleared all {} mappings from database", deleted);
        Ok(deleted)
    }

    fn hash_value(&self, value: &str) -> String {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        
        let mut hasher = DefaultHasher::new();
        value.hash(&mut hasher);
        format!("{:x}", hasher.finish())
    }
}

#[derive(Debug)]
pub struct MappingStatistics {
    pub total_mappings: usize,
    pub total_cache_entries: usize,
    pub mappings_by_type: HashMap<String, usize>,
    pub oldest_mapping_age: Option<u64>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AnonymizedEntity;
    use std::path::PathBuf;
    use tempfile::TempDir;

    fn create_test_config() -> (MappingConfig, TempDir) {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test_mappings.db");
        
        let config = MappingConfig {
            database_path: db_path,
            encryption: false,
            retention_days: Some(30),
        };
        
        (config, temp_dir)
    }

    fn create_test_entity() -> AnonymizedEntity {
        AnonymizedEntity {
            entity_type: "email".to_string(),
            original_value: "john@example.com".to_string(),
            fake_value: "fake@company.com".to_string(),
            mapping_id: Uuid::new_v4().to_string(),
        }
    }

    #[test]
    fn test_mapping_store_creation() {
        let (config, _temp_dir) = create_test_config();
        let store = MappingStore::new(config).unwrap();
        assert!(store.conn.prepare("SELECT COUNT(*) FROM entity_mappings").is_ok());
    }

    #[test]
    fn test_in_memory_database() {
        let config = MappingConfig {
            database_path: PathBuf::from(":memory:"),
            encryption: false,
            retention_days: None,
        };
        
        let store = MappingStore::new(config).unwrap();
        assert!(store.conn.prepare("SELECT COUNT(*) FROM entity_mappings").is_ok());
    }

    #[test]
    fn test_store_and_retrieve_mapping() {
        let (config, _temp_dir) = create_test_config();
        let mut store = MappingStore::new(config).unwrap();
        
        let entity = create_test_entity();
        
        // Store mapping
        store.store_mapping(&entity).unwrap();
        
        // Retrieve mapping
        let retrieved = store.get_mapping("email", "john@example.com").unwrap();
        assert_eq!(retrieved, Some("fake@company.com".to_string()));
        let non_existent = store.get_mapping("email", "nonexistent@example.com").unwrap();
        assert_eq!(non_existent, None);
    }

    #[test]
    fn test_mapping_consistency() {
        let (config, _temp_dir) = create_test_config();
        let mut store = MappingStore::new(config).unwrap();
        
        let entity1 = AnonymizedEntity {
            entity_type: "email".to_string(),
            original_value: "same@example.com".to_string(),
            fake_value: "first@company.com".to_string(),
            mapping_id: Uuid::new_v4().to_string(),
        };
        
        let entity2 = AnonymizedEntity {
            entity_type: "email".to_string(),
            original_value: "same@example.com".to_string(),
            fake_value: "second@company.com".to_string(),
            mapping_id: Uuid::new_v4().to_string(),
        };
        
        // Store first mapping
        store.store_mapping(&entity1).unwrap();
        store.store_mapping(&entity2).unwrap();
        
        let retrieved = store.get_mapping("email", "same@example.com").unwrap();
        assert_eq!(retrieved, Some("first@company.com".to_string()));
    }

    #[test]
    fn test_batch_operations() {
        let (config, _temp_dir) = create_test_config();
        let mut store = MappingStore::new(config).unwrap();
        
        let entities = vec![
            AnonymizedEntity {
                entity_type: "email".to_string(),
                original_value: "batch1@example.com".to_string(),
                fake_value: "fake1@company.com".to_string(),
                mapping_id: Uuid::new_v4().to_string(),
            },
            AnonymizedEntity {
                entity_type: "phone".to_string(),
                original_value: "555-123-4567".to_string(),
                fake_value: "555-987-6543".to_string(),
                mapping_id: Uuid::new_v4().to_string(),
            },
        ];
        
        // Store batch
        store.store_mappings_batch(&entities).unwrap();
        
        // Prepare batch retrieval requests
        let requests = vec![
            ("email".to_string(), "batch1@example.com".to_string()),
            ("phone".to_string(), "555-123-4567".to_string()),
            ("email".to_string(), "nonexistent@example.com".to_string()),
        ];
        
        // Retrieve batch
        let results = store.get_mappings_batch(&requests).unwrap();
        
        assert_eq!(results.len(), 2);
        assert_eq!(results.get("batch1@example.com"), Some(&"fake1@company.com".to_string()));
        assert_eq!(results.get("555-123-4567"), Some(&"555-987-6543".to_string()));
        assert!(!results.contains_key("nonexistent@example.com"));
    }

    #[test]
    fn test_statistics() {
        let (config, _temp_dir) = create_test_config();
        let mut store = MappingStore::new(config).unwrap();
        
        let entities = vec![
            AnonymizedEntity {
                entity_type: "email".to_string(),
                original_value: "stats1@example.com".to_string(),
                fake_value: "fake1@company.com".to_string(),
                mapping_id: Uuid::new_v4().to_string(),
            },
            AnonymizedEntity {
                entity_type: "email".to_string(),
                original_value: "stats2@example.com".to_string(),
                fake_value: "fake2@company.com".to_string(),
                mapping_id: Uuid::new_v4().to_string(),
            },
            AnonymizedEntity {
                entity_type: "phone".to_string(),
                original_value: "555-111-2222".to_string(),
                fake_value: "555-999-8888".to_string(),
                mapping_id: Uuid::new_v4().to_string(),
            },
        ];
        
        store.store_mappings_batch(&entities).unwrap();
        
        let stats = store.get_statistics().unwrap();
        
        assert_eq!(stats.total_mappings, 3);
        assert_eq!(stats.total_cache_entries, 0);
        assert_eq!(stats.mappings_by_type.get("email"), Some(&2));
        assert_eq!(stats.mappings_by_type.get("phone"), Some(&1));
        assert!(stats.oldest_mapping_age.is_some());
    }

    #[test]
    fn test_clear_all_mappings() {
        let (config, _temp_dir) = create_test_config();
        let mut store = MappingStore::new(config).unwrap();
        
        let entity = create_test_entity();
        store.store_mapping(&entity).unwrap();
        
        // Verify mapping exists
        let retrieved = store.get_mapping("email", "john@example.com").unwrap();
        assert!(retrieved.is_some());
        
        // Clear all mappings
        let deleted = store.clear_all_mappings().unwrap();
        assert_eq!(deleted, 1);
        
        // Verify mapping is gone
        let retrieved = store.get_mapping("email", "john@example.com").unwrap();
        assert!(retrieved.is_none());
    }

    #[test]
    fn test_hash_consistency() {
        let (config, _temp_dir) = create_test_config();
        let store = MappingStore::new(config).unwrap();
        
        let hash1 = store.hash_value("test@example.com");
        let hash2 = store.hash_value("test@example.com");
        let hash3 = store.hash_value("different@example.com");
        
        assert_eq!(hash1, hash2);
        assert_ne!(hash1, hash3);
    }

    #[test]
    fn test_different_entity_types() {
        let (config, _temp_dir) = create_test_config();
        let mut store = MappingStore::new(config).unwrap();
        let email_entity = AnonymizedEntity {
            entity_type: "email".to_string(),
            original_value: "john@example.com".to_string(),
            fake_value: "fake@company.com".to_string(),
            mapping_id: Uuid::new_v4().to_string(),
        };
        
        let name_entity = AnonymizedEntity {
            entity_type: "name".to_string(),
            original_value: "john@example.com".to_string(),
            fake_value: "Jane Doe".to_string(),
            mapping_id: Uuid::new_v4().to_string(),
        };
        
        store.store_mapping(&email_entity).unwrap();
        store.store_mapping(&name_entity).unwrap();
        
        let email_result = store.get_mapping("email", "john@example.com").unwrap();
        let name_result = store.get_mapping("name", "john@example.com").unwrap();
        
        assert_eq!(email_result, Some("fake@company.com".to_string()));
        assert_eq!(name_result, Some("Jane Doe".to_string()));
    }

    #[test]
    fn test_llm_cache_store_and_retrieve() {
        let (config, _temp_dir) = create_test_config();
        let mut store = MappingStore::new(config).unwrap();
        
        let text = "Contact Sarah Johnson at sarah@company.com";
        let entities = vec![
            DetectedEntity {
                entity_type: "person_name".to_string(),
                original_value: "Sarah Johnson".to_string(),
                start: 8,
                end: 21,
                confidence: 0.95,
            },
            DetectedEntity {
                entity_type: "email".to_string(),
                original_value: "sarah@company.com".to_string(),
                start: 25,
                end: 42,
                confidence: 0.98,
            },
        ];
        let model_name = "llama3.2:3b";

        store.store_llm_cache(text, &entities, model_name).unwrap();

        // Retrieve cache entry
        let cached_entities = store.get_llm_cache(text, model_name).unwrap();
        assert!(cached_entities.is_some());
        
        let cached_entities = cached_entities.unwrap();
        assert_eq!(cached_entities.len(), 2);
        assert_eq!(cached_entities[0].entity_type, "person_name");
        assert_eq!(cached_entities[0].original_value, "Sarah Johnson");
        assert_eq!(cached_entities[1].entity_type, "email");
        assert_eq!(cached_entities[1].original_value, "sarah@company.com");
    }

    #[test]
    fn test_llm_cache_miss() {
        let (config, _temp_dir) = create_test_config();
        let store = MappingStore::new(config).unwrap();
        
        let result = store.get_llm_cache("Non-existent text", "llama3.2:3b").unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_llm_cache_different_models() {
        let (config, _temp_dir) = create_test_config();
        let mut store = MappingStore::new(config).unwrap();
        
        let text = "John Doe works at ACME Corp";
        let entities1 = vec![DetectedEntity {
            entity_type: "person_name".to_string(),
            original_value: "John Doe".to_string(),
            start: 0,
            end: 8,
            confidence: 0.9,
        }];
        let entities2 = vec![DetectedEntity {
            entity_type: "organization".to_string(),
            original_value: "ACME Corp".to_string(),
            start: 19,
            end: 28,
            confidence: 0.85,
        }];

        store.store_llm_cache(text, &entities1, "model1").unwrap();
        store.store_llm_cache(text, &entities2, "model2").unwrap();

        let result1 = store.get_llm_cache(text, "model1").unwrap().unwrap();
        let result2 = store.get_llm_cache(text, "model2").unwrap().unwrap();

        assert_eq!(result1.len(), 1);
        assert_eq!(result1[0].entity_type, "person_name");
        
        assert_eq!(result2.len(), 1);
        assert_eq!(result2[0].entity_type, "organization");
    }

    #[test]
    fn test_llm_cache_replacement() {
        let (config, _temp_dir) = create_test_config();
        let mut store = MappingStore::new(config).unwrap();
        
        let text = "Replace this content";
        let model_name = "test-model";
        
        let entities1 = vec![DetectedEntity {
            entity_type: "old_type".to_string(),
            original_value: "old_value".to_string(),
            start: 0,
            end: 3,
            confidence: 0.5,
        }];
        
        let entities2 = vec![DetectedEntity {
            entity_type: "new_type".to_string(),
            original_value: "new_value".to_string(),
            start: 0,
            end: 3,
            confidence: 0.9,
        }];

        store.store_llm_cache(text, &entities1, model_name).unwrap();
        
        store.store_llm_cache(text, &entities2, model_name).unwrap();
        let result = store.get_llm_cache(text, model_name).unwrap().unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].entity_type, "new_type");
        assert_eq!(result[0].original_value, "new_value");
    }

    #[test]
    fn test_llm_cache_statistics() {
        let (config, _temp_dir) = create_test_config();
        let mut store = MappingStore::new(config).unwrap();
        let entity = create_test_entity();
        store.store_mapping(&entity).unwrap();
        
        let text = "Cache this text";
        let entities = vec![DetectedEntity {
            entity_type: "test".to_string(),
            original_value: "test".to_string(),
            start: 0,
            end: 4,
            confidence: 0.8,
        }];
        store.store_llm_cache(text, &entities, "test-model").unwrap();

        let stats = store.get_statistics().unwrap();
        assert_eq!(stats.total_mappings, 1);
        assert_eq!(stats.total_cache_entries, 1);
    }

    #[test]
    fn test_clear_llm_cache() {
        let (config, _temp_dir) = create_test_config();
        let mut store = MappingStore::new(config).unwrap();
        
        let text = "Text to cache";
        let entities = vec![DetectedEntity {
            entity_type: "test".to_string(),
            original_value: "test".to_string(),
            start: 0,
            end: 4,
            confidence: 0.9,
        }];
        
        store.store_llm_cache(text, &entities, "test-model").unwrap();
        
        // Verify it exists
        let result = store.get_llm_cache(text, "test-model").unwrap();
        assert!(result.is_some());
        
        // Clear cache
        let deleted = store.clear_llm_cache().unwrap();
        assert_eq!(deleted, 1);
        
        // Verify it's gone
        let result = store.get_llm_cache(text, "test-model").unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_cleanup_expired_entries_with_cache() {
        let (mut config, _temp_dir) = create_test_config();
        config.retention_days = Some(0); // Expire immediately
        
        let mut store = MappingStore::new(config).unwrap();
        
        // Store mapping and cache entry
        let entity = create_test_entity();
        store.store_mapping(&entity).unwrap();
        
        let text = "Text to cache";
        let entities = vec![DetectedEntity {
            entity_type: "test".to_string(),
            original_value: "test".to_string(),
            start: 0,
            end: 4,
            confidence: 0.9,
        }];
        store.store_llm_cache(text, &entities, "test-model").unwrap();
        
        std::thread::sleep(std::time::Duration::from_secs(1));
        
        let deleted = store.cleanup_expired_mappings().unwrap();
        assert_eq!(deleted, 2);
        
        let mapping_result = store.get_mapping("email", "john@example.com").unwrap();
        assert!(mapping_result.is_none());
        
        let cache_result = store.get_llm_cache(text, "test-model").unwrap();
        assert!(cache_result.is_none());
    }

    #[test]
    fn test_llm_cache_with_empty_entities() {
        let (config, _temp_dir) = create_test_config();
        let mut store = MappingStore::new(config).unwrap();
        
        let text = "No PII in this text";
        let empty_entities: Vec<DetectedEntity> = vec![];
        let model_name = "test-model";

        store.store_llm_cache(text, &empty_entities, model_name).unwrap();

        // Retrieve cache entry
        let cached_entities = store.get_llm_cache(text, model_name).unwrap();
        assert!(cached_entities.is_some());
        
        let cached_entities = cached_entities.unwrap();
        assert_eq!(cached_entities.len(), 0);
    }

    #[test]
    fn test_llm_cache_with_large_text_and_many_entities() {
        let (config, _temp_dir) = create_test_config();
        let mut store = MappingStore::new(config).unwrap();
        
        let text = "Contact John Smith at john@company.com, Jane Doe at jane@example.org, Bob Wilson at bob@test.net, and Alice Brown at alice@sample.com. Call (555) 123-4567 or (555) 987-6543 for more information.";
        let entities = vec![
            DetectedEntity {
                entity_type: "person_name".to_string(),
                original_value: "John Smith".to_string(),
                start: 8,
                end: 18,
                confidence: 0.95,
            },
            DetectedEntity {
                entity_type: "email".to_string(),
                original_value: "john@company.com".to_string(),
                start: 22,
                end: 38,
                confidence: 0.98,
            },
            DetectedEntity {
                entity_type: "person_name".to_string(),
                original_value: "Jane Doe".to_string(),
                start: 40,
                end: 48,
                confidence: 0.93,
            },
            DetectedEntity {
                entity_type: "email".to_string(),
                original_value: "jane@example.org".to_string(),
                start: 52,
                end: 68,
                confidence: 0.97,
            },
            DetectedEntity {
                entity_type: "phone".to_string(),
                original_value: "(555) 123-4567".to_string(),
                start: 150,
                end: 164,
                confidence: 0.99,
            },
        ];
        let model_name = "llama3.2:3b";

        store.store_llm_cache(text, &entities, model_name).unwrap();

        let cached_entities = store.get_llm_cache(text, model_name).unwrap().unwrap();
        assert_eq!(cached_entities.len(), 5);
        
        assert_eq!(cached_entities[0].entity_type, "person_name");
        assert_eq!(cached_entities[0].original_value, "John Smith");
        assert_eq!(cached_entities[1].entity_type, "email");
        assert_eq!(cached_entities[1].original_value, "john@company.com");
        assert_eq!(cached_entities[4].entity_type, "phone");
        assert_eq!(cached_entities[4].original_value, "(555) 123-4567");
    }

    #[test]
    fn test_llm_cache_with_special_characters_and_unicode() {
        let (config, _temp_dir) = create_test_config();
        let mut store = MappingStore::new(config).unwrap();
        
        let text = "Contactez François Müller à françois.müller@société.com ou José García à josé@españa.es";
        let entities = vec![
            DetectedEntity {
                entity_type: "person_name".to_string(),
                original_value: "François Müller".to_string(),
                start: 10,
                end: 25,
                confidence: 0.92,
            },
            DetectedEntity {
                entity_type: "email".to_string(),
                original_value: "françois.müller@société.com".to_string(),
                start: 28,
                end: 55,
                confidence: 0.96,
            },
            DetectedEntity {
                entity_type: "person_name".to_string(),
                original_value: "José García".to_string(),
                start: 59,
                end: 70,
                confidence: 0.94,
            },
        ];
        let model_name = "llama3.2:3b";

        store.store_llm_cache(text, &entities, model_name).unwrap();

        let cached_entities = store.get_llm_cache(text, model_name).unwrap().unwrap();
        assert_eq!(cached_entities.len(), 3);
        assert_eq!(cached_entities[0].original_value, "François Müller");
        assert_eq!(cached_entities[1].original_value, "françois.müller@société.com");
        assert_eq!(cached_entities[2].original_value, "José García");
    }

    #[test]
    fn test_llm_cache_concurrent_model_operations() {
        let (config, _temp_dir) = create_test_config();
        let mut store = MappingStore::new(config).unwrap();
        
        let base_text = "Test concurrent operations with different models";
        let models = ["model-a", "model-b", "model-c", "model-d", "model-e"];
        
        for (i, model) in models.iter().enumerate() {
            let entities = vec![DetectedEntity {
                entity_type: format!("entity_type_{}", i),
                original_value: format!("value_{}", i),
                start: i * 5,
                end: (i + 1) * 5,
                confidence: 0.8 + (i as f64 * 0.02),
            }];
            
            store.store_llm_cache(base_text, &entities, model).unwrap();
        }

        for (i, model) in models.iter().enumerate() {
            let cached_entities = store.get_llm_cache(base_text, model).unwrap().unwrap();
            assert_eq!(cached_entities.len(), 1);
            assert_eq!(cached_entities[0].entity_type, format!("entity_type_{}", i));
            assert_eq!(cached_entities[0].original_value, format!("value_{}", i));
            assert_eq!(cached_entities[0].start, i * 5);
            assert_eq!(cached_entities[0].end, (i + 1) * 5);
        }

        let stats = store.get_statistics().unwrap();
        assert_eq!(stats.total_cache_entries, 5);
    }

    #[test]
    fn test_llm_cache_persistence_across_store_recreations() {
        let (config, temp_dir) = create_test_config();
        let _db_path = config.database_path.clone();
        
        let text = "Persistent cache test with John Doe";
        let entities = vec![DetectedEntity {
            entity_type: "person_name".to_string(),
            original_value: "John Doe".to_string(),
            start: 28,
            end: 36,
            confidence: 0.95,
        }];
        let model_name = "persistent-model";

        {
            let mut store1 = MappingStore::new(config.clone()).unwrap();
            store1.store_llm_cache(text, &entities, model_name).unwrap();
            
            let cached = store1.get_llm_cache(text, model_name).unwrap().unwrap();
            assert_eq!(cached.len(), 1);
            assert_eq!(cached[0].original_value, "John Doe");
        }

        {
            let store2 = MappingStore::new(config.clone()).unwrap();
            
            let cached = store2.get_llm_cache(text, model_name).unwrap().unwrap();
            assert_eq!(cached.len(), 1);
            assert_eq!(cached[0].entity_type, "person_name");
            assert_eq!(cached[0].original_value, "John Doe");
            assert_eq!(cached[0].confidence, 0.95);
            
            let stats = store2.get_statistics().unwrap();
            assert_eq!(stats.total_cache_entries, 1);
        }

        drop(temp_dir);
    }
}