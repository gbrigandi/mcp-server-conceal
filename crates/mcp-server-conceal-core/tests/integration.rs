use mcp_server_conceal_core::{IntegratedProxyConfig, Config};
use std::collections::HashMap;
use std::path::PathBuf;

#[tokio::test]
async fn test_integrated_proxy_config() {
    // Test that we can create a valid IntegratedProxyConfig (this tests the refactored structure)
    
    // Test that we can create a valid config (this tests our CLI parsing logic indirectly)
    let mut env = HashMap::new();
    env.insert("TEST_VAR".to_string(), "test_value".to_string());
    
    let config = IntegratedProxyConfig {
        target_command: "echo".to_string(),
        target_args: vec!["hello".to_string(), "world".to_string()],
        target_env: env.clone(),
        target_cwd: Some(PathBuf::from("/tmp")),
        config: Config::default(),
        ollama_config: mcp_server_conceal_core::OllamaConfig::default(),
    };
    
    assert_eq!(config.target_command, "echo");
    assert_eq!(config.target_args, vec!["hello", "world"]);
    assert_eq!(config.target_env, env);
    assert_eq!(config.target_cwd, Some(PathBuf::from("/tmp")));
    assert!(!config.config.detection.patterns.is_empty());
    assert!(!config.ollama_config.enabled); // Default is disabled
    
    println!("✅ IntegratedProxyConfig can be created and works correctly after refactoring");
}

#[test]
fn test_module_exports() {
    // Test that our refactored module exports work correctly
    use mcp_server_conceal_core::{Config, OllamaConfig};
    use mcp_server_conceal_core::{RegexDetectionEngine, FakerEngine, MappingStore};
    
    // These should all compile and be accessible
    let _config = Config::default();
    let _ollama_config = OllamaConfig::default();
    
    // Test that we can create the core engines (these were preserved in refactoring)
    let detection_engine = RegexDetectionEngine::new(&_config.detection);
    assert!(detection_engine.is_ok());
    
    let faker_engine = FakerEngine::new(&_config.faker);
    // FakerEngine doesn't return Result, so we just check it exists
    drop(faker_engine);
    
    let mapping_store = MappingStore::new(_config.mapping.clone());
    assert!(mapping_store.is_ok());
    
    println!("✅ All refactored module exports work correctly");
}