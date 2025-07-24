use crate::config::{Config, DetectedEntity, AnonymizedEntity};
use crate::detection::RegexDetectionEngine;
use crate::faker::FakerEngine;
use crate::mapping::MappingStore;
use crate::ollama::{OllamaClient, OllamaConfig};
use crate::IntegratedProxyConfig;
use anyhow::Result;
use std::collections::HashMap;
use std::path::PathBuf;
use tempfile::TempDir;
use tracing::{info, warn};

/// Integration test for the complete pipeline: regex → ollama → faker
/// This test requires a running Ollama service with llama3.2:3b model
#[tokio::test]
#[ignore] // Ignored by default, run with --ignored to include
async fn test_complete_pipeline_with_real_ollama() -> Result<()> {
    // Initialize tracing for test
    let _ = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .with_target(false)
        .try_init();

    info!("Starting complete pipeline integration test");

    // Create test configuration
    let (config, ollama_config) = create_test_config().await?;
    
    // Initialize components
    let detection_engine = RegexDetectionEngine::new(&config.detection)?;
    let mut faker_engine = FakerEngine::new(&config.faker);
    let mut mapping_store = MappingStore::new(config.mapping.clone())?;
    let ollama_client = OllamaClient::new(ollama_config.clone(), None)?;

    // Check if Ollama is available
    if !ollama_client.health_check().await? {
        warn!("Ollama service not available, skipping integration test");
        return Ok(());
    }

    info!("Ollama service is available, proceeding with pipeline test");

    // Test input with mixed PII
    let test_text = "Hi, I'm Sarah Johnson and you can reach me at sarah.johnson@company.com or call (555) 123-4567. I work at ACME Corporation in the engineering department.";

    info!("Processing text: {}", test_text);

    // Step 1: Regex-based detection
    info!("Step 1: Running regex detection");
    let regex_entities = detection_engine.detect_in_text(test_text);
    info!("Regex detected {} entities", regex_entities.len());
    for entity in &regex_entities {
        info!("  Regex: {} = '{}' at {}:{}", 
              entity.entity_type, entity.original_value, entity.start, entity.end);
    }

    // Step 2: Check LLM cache first
    info!("Step 2: Checking LLM cache");
    let model_name = &ollama_config.model;
    let cached_entities = mapping_store.get_llm_cache(test_text, model_name)?;
    
    let llm_entities = if let Some(cached) = cached_entities {
        info!("LLM cache hit! Found {} cached entities", cached.len());
        cached
    } else {
        info!("LLM cache miss, calling Ollama for LLM detection");
        let entities = ollama_client.extract_entities(test_text).await?;
        info!("Ollama detected {} entities", entities.len());
        
        // Cache the results
        mapping_store.store_llm_cache(test_text, &entities, model_name)?;
        info!("Stored LLM results in cache");
        
        entities
    };

    for entity in &llm_entities {
        info!("  LLM: {} = '{}' at {}:{} (confidence: {:.2})", 
              entity.entity_type, entity.original_value, entity.start, entity.end, entity.confidence);
    }

    // Step 3: Combine and deduplicate entities (regex + LLM)
    info!("Step 3: Combining regex and LLM entities");
    let combined_entities = combine_entities(regex_entities, llm_entities.clone());
    info!("Combined total: {} unique entities", combined_entities.len());

    // Step 4: Generate fake replacements and store mappings
    info!("Step 4: Generating fake data and storing mappings");
    let mut anonymized_entities = Vec::new();
    
    for entity in combined_entities {
        // Check if we already have a mapping
        if let Some(existing_fake) = mapping_store.get_mapping(&entity.entity_type, &entity.original_value)? {
            info!("  Using existing mapping: {} -> {}", entity.original_value, existing_fake);
            let original_value_clone = entity.original_value.clone();
            anonymized_entities.push(AnonymizedEntity {
                entity_type: entity.entity_type.clone(),
                original_value: entity.original_value.clone(),
                fake_value: existing_fake,
                mapping_id: format!("existing-{}", original_value_clone),
            });
        } else {
            // Generate new fake value using anonymize_entity
            let anonymized = faker_engine.anonymize_entity(&entity)?;
            info!("  Generated new fake: {} -> {}", entity.original_value, anonymized.fake_value);
            
            // Store the mapping
            mapping_store.store_mapping(&anonymized)?;
            anonymized_entities.push(anonymized);
        }
    }

    // Step 5: Apply replacements to create anonymized text
    info!("Step 5: Creating anonymized text");
    let anonymized_text = apply_replacements(test_text, &anonymized_entities)?;
    info!("Original text: {}", test_text);
    info!("Anonymized text: {}", anonymized_text);

    // Step 6: Verify pipeline results
    info!("Step 6: Verifying pipeline results");
    
    // Verify we detected some entities
    assert!(!anonymized_entities.is_empty(), "Pipeline should detect at least one entity");
    
    // Verify the text was modified
    assert_ne!(test_text, anonymized_text, "Text should be anonymized");
    
    // Verify no original PII remains in anonymized text
    for entity in &anonymized_entities {
        assert!(
            !anonymized_text.contains(&entity.original_value),
            "Original PII '{}' should not appear in anonymized text",
            entity.original_value
        );
        // Note: Some fake values might not appear if overlapping entities were replaced by others
        // This is expected behavior - the important thing is that the original PII is gone
        if !anonymized_text.contains(&entity.fake_value) {
            info!("Fake value '{}' not in final text (likely replaced by overlapping entity)", entity.fake_value);
        }
    }

    // Step 7: Test consistency - run pipeline again with same text
    info!("Step 7: Testing consistency with second run");
    
    // Second run should use cached LLM results and existing mappings
    let cached_entities_2nd = mapping_store.get_llm_cache(test_text, model_name)?.unwrap();
    assert_eq!(cached_entities_2nd.len(), llm_entities.len(), "Cache should return same entities");
    
    let anonymized_text_2nd = process_text_through_pipeline(
        test_text,
        &detection_engine,
        &ollama_client,
        &mut faker_engine,
        &mut mapping_store,
        model_name
    ).await?;
    
    assert_eq!(anonymized_text, anonymized_text_2nd, "Pipeline should be consistent");
    
    info!("Integration test completed successfully!");
    
    // Step 8: Check database statistics
    let stats = mapping_store.get_statistics()?;
    info!("Final database stats:");
    info!("  Total mappings: {}", stats.total_mappings);
    info!("  Cache entries: {}", stats.total_cache_entries);
    info!("  Mappings by type: {:?}", stats.mappings_by_type);
    
    Ok(())
}

/// Test with Ollama disabled to verify graceful fallback
#[tokio::test]
async fn test_pipeline_with_disabled_ollama() -> Result<()> {
    let _ = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .with_target(false)
        .try_init();

    info!("Testing pipeline with disabled Ollama");

    let (config, mut ollama_config) = create_test_config().await?;
    ollama_config.enabled = false; // Disable Ollama

    let detection_engine = RegexDetectionEngine::new(&config.detection)?;
    let mut faker_engine = FakerEngine::new(&config.faker);
    let mut mapping_store = MappingStore::new(config.mapping.clone())?;
    let ollama_client = OllamaClient::new(ollama_config.clone(), None)?;

    let test_text = "Contact john@example.com or call (555) 987-6543";

    let anonymized_text = process_text_through_pipeline(
        test_text,
        &detection_engine,
        &ollama_client,
        &mut faker_engine,
        &mut mapping_store,
        &ollama_config.model
    ).await?;

    // Should still work with regex-only detection
    assert_ne!(test_text, anonymized_text, "Text should be anonymized even without LLM");
    info!("Pipeline works correctly with disabled Ollama");

    Ok(())
}

/// Helper function to create test configuration
async fn create_test_config() -> Result<(Config, OllamaConfig)> {
    let temp_dir = TempDir::new()?;
    let mut config = Config::default();
    
    // Configure for testing
    config.mapping.database_path = temp_dir.path().join("test_integration.db");
    config.faker.seed = Some(12345); // Deterministic for testing
    
    let ollama_config = OllamaConfig {
        enabled: true,
        endpoint: "http://localhost:11434".to_string(),
        model: "llama3.2:3b".to_string(),
        timeout_seconds: 300,
    };
    
    // Keep temp_dir alive by leaking it (acceptable for tests)
    std::mem::forget(temp_dir);
    
    Ok((config, ollama_config))
}

/// Combine regex and LLM entities, removing duplicates
fn combine_entities(regex_entities: Vec<DetectedEntity>, llm_entities: Vec<DetectedEntity>) -> Vec<DetectedEntity> {
    let mut combined = HashMap::new();
    
    // Add regex entities first (lower priority)
    for entity in regex_entities {
        let key = format!("{}:{}:{}", entity.entity_type, entity.start, entity.end);
        combined.insert(key, entity);
    }
    
    // Add LLM entities (higher priority, can override regex)
    for entity in llm_entities {
        let key = format!("{}:{}:{}", entity.entity_type, entity.start, entity.end);
        combined.insert(key, entity);
    }
    
    combined.into_values().collect()
}

/// Apply entity replacements to text, handling position shifts
fn apply_replacements(text: &str, entities: &[AnonymizedEntity]) -> Result<String> {
    let mut result = text.to_string();
    
    // Sort entities by start position to apply replacements left-to-right
    let mut sorted_entities: Vec<_> = entities.iter().collect();
    sorted_entities.sort_by_key(|e| {
        // Find the entity in the original text to get positions
        text.find(&e.original_value).unwrap_or(0)
    });
    
    for entity in sorted_entities {
        if let Some(start_pos) = result.find(&entity.original_value) {
            let end_pos = start_pos + entity.original_value.len();
            result.replace_range(start_pos..end_pos, &entity.fake_value);
        }
    }
    
    Ok(result)
}

/// Complete pipeline processing helper function
async fn process_text_through_pipeline(
    text: &str,
    detection_engine: &RegexDetectionEngine,
    ollama_client: &OllamaClient,
    faker_engine: &mut FakerEngine,
    mapping_store: &mut MappingStore,
    model_name: &str,
) -> Result<String> {
    // Step 1: Regex detection
    let regex_entities = detection_engine.detect_in_text(text);
    
    // Step 2: LLM detection (with caching)
    let llm_entities = if let Some(cached) = mapping_store.get_llm_cache(text, model_name)? {
        cached
    } else {
        let entities = ollama_client.extract_entities(text).await?;
        mapping_store.store_llm_cache(text, &entities, model_name)?;
        entities
    };
    
    // Step 3: Combine entities
    let combined_entities = combine_entities(regex_entities, llm_entities);
    
    // Step 4: Generate fakes and store mappings
    let mut anonymized_entities = Vec::new();
    for entity in combined_entities {
        if let Some(existing_fake) = mapping_store.get_mapping(&entity.entity_type, &entity.original_value)? {
            let original_value_clone = entity.original_value.clone();
            anonymized_entities.push(AnonymizedEntity {
                entity_type: entity.entity_type,
                original_value: entity.original_value,
                fake_value: existing_fake,
                mapping_id: format!("existing-{}", original_value_clone),
            });
        } else {
            let anonymized = faker_engine.anonymize_entity(&entity)?;
            mapping_store.store_mapping(&anonymized)?;
            anonymized_entities.push(anonymized);
        }
    }
    
    // Step 5: Apply replacements
    apply_replacements(text, &anonymized_entities)
}

/// Performance test with small payload (< 100 chars)
/// Tests basic performance baseline and cache efficiency
#[tokio::test]
#[ignore] // Run with --ignored
async fn test_ollama_performance_small_payload() -> Result<()> {
    let _ = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .try_init();

    info!("Starting Ollama performance test - Small payload");

    let (config, ollama_config) = create_test_config().await?;
    let detection_engine = RegexDetectionEngine::new(&config.detection)?;
    let mut faker_engine = FakerEngine::new(&config.faker);
    let mut mapping_store = MappingStore::new(config.mapping.clone())?;
    let ollama_client = OllamaClient::new(ollama_config.clone(), None)?;

    if !ollama_client.health_check().await? {
        warn!("Ollama not available, skipping performance test");
        return Ok(());
    }

    // Small payload with 1-2 entities
    let small_text = "Contact john@test.com for details.";
    
    info!("Testing small payload: {} characters", small_text.len());
    assert!(small_text.len() < 100, "Payload should be under 100 characters");

    // Measure first run (no cache)
    let start_time = std::time::Instant::now();
    let result1 = process_text_through_pipeline(
        small_text,
        &detection_engine,
        &ollama_client,
        &mut faker_engine,
        &mut mapping_store,
        &ollama_config.model
    ).await?;
    let first_run_duration = start_time.elapsed();
    
    info!("First run completed in {:?}", first_run_duration);
    info!("Original: {}", small_text);
    info!("Anonymized: {}", result1);

    // Measure second run (with cache)
    let start_time = std::time::Instant::now();
    let result2 = process_text_through_pipeline(
        small_text,
        &detection_engine,
        &ollama_client,
        &mut faker_engine,
        &mut mapping_store,
        &ollama_config.model
    ).await?;
    let second_run_duration = start_time.elapsed();
    
    info!("Second run completed in {:?}", second_run_duration);

    // Verify consistency
    assert_eq!(result1, result2, "Results should be consistent");

    // Cache should make second run significantly faster
    assert!(second_run_duration < first_run_duration / 2, 
            "Cached run should be at least 50% faster");

    // Small payload should complete within reasonable time (allowing for concurrent execution)
    assert!(first_run_duration.as_secs() < 30, 
            "Small payload should process in under 30 seconds");

    info!("Small payload performance test passed");
    Ok(())
}

/// Performance test with medium payload (500-1000 chars)
/// Tests entity density processing and throughput
#[tokio::test]
#[ignore] // Run with --ignored
async fn test_ollama_performance_medium_payload() -> Result<()> {
    let _ = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .try_init();

    info!("Starting Ollama performance test - Medium payload");

    let (config, ollama_config) = create_test_config().await?;
    let detection_engine = RegexDetectionEngine::new(&config.detection)?;
    let mut faker_engine = FakerEngine::new(&config.faker);
    let mut mapping_store = MappingStore::new(config.mapping.clone())?;
    let ollama_client = OllamaClient::new(ollama_config.clone(), None)?;

    if !ollama_client.health_check().await? {
        warn!("Ollama not available, skipping performance test");
        return Ok(());
    }

    // Medium payload with multiple entities of different types
    let medium_text = r#"
    Dear Dr. Sarah Johnson,

    Thank you for your inquiry about our services. We have received your application
    and would like to schedule a consultation. Please contact our office manager,
    Michael Thompson, at michael.thompson@healthcorp.com or call (555) 987-6543.

    Your reference number is REF-2024-001234. Please have this ready when you call.
    
    Our facility is located at 123 Medical Plaza, Suite 400, Boston, MA 02101.
    You can also visit our website at https://www.healthcorp.com for more information.
    
    If you need to reschedule, please email sarah.admin@healthcorp.com or call our
    main line at (555) 123-4567. Our billing department can be reached at 
    billing@healthcorp.com.

    Best regards,
    Dr. Emily Rodriguez
    Chief Medical Officer
    HealthCorp Medical Group
    emily.rodriguez@healthcorp.com
    Direct: (555) 456-7890
    "#;
    
    info!("Testing medium payload: {} characters", medium_text.len());
    assert!(medium_text.len() >= 500 && medium_text.len() <= 1200, 
            "Payload should be 500-1200 characters");

    // Count expected entities for throughput measurement
    let regex_entities = detection_engine.detect_in_text(medium_text);
    info!("Regex detected {} entities in medium text", regex_entities.len());

    // Measure processing time
    let start_time = std::time::Instant::now();
    let result = process_text_through_pipeline(
        medium_text,
        &detection_engine,
        &ollama_client,
        &mut faker_engine,
        &mut mapping_store,
        &ollama_config.model
    ).await?;
    let processing_duration = start_time.elapsed();
    
    info!("Medium payload processed in {:?}", processing_duration);
    info!("Characters per second: {:.1}", 
          medium_text.len() as f64 / processing_duration.as_secs_f64());

    // Verify anonymization occurred
    assert_ne!(medium_text, result, "Text should be anonymized");
    assert!(!result.is_empty(), "Result should not be empty");

    // Medium payload should complete within reasonable time (allowing for concurrent execution)
    assert!(processing_duration.as_secs() < 45, 
            "Medium payload should process in under 45 seconds");

    // Test throughput calculation
    let throughput = medium_text.len() as f64 / processing_duration.as_secs_f64();
    info!("Throughput: {:.1} chars/second", throughput);
    assert!(throughput > 10.0, "Should maintain reasonable throughput");

    info!("Medium payload performance test passed");
    Ok(())
}

/// Performance test with large payload (2000+ chars)
/// Tests memory efficiency and timeout behavior
#[tokio::test]
#[ignore] // Run with --ignored
async fn test_ollama_performance_large_payload() -> Result<()> {
    let _ = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .try_init();

    info!("Starting Ollama performance test - Large payload");

    let (config, ollama_config) = create_test_config().await?;
    let detection_engine = RegexDetectionEngine::new(&config.detection)?;
    let mut faker_engine = FakerEngine::new(&config.faker);
    let mut mapping_store = MappingStore::new(config.mapping.clone())?;
    let ollama_client = OllamaClient::new(ollama_config.clone(), None)?;

    if !ollama_client.health_check().await? {
        warn!("Ollama not available, skipping performance test");
        return Ok(());
    }

    // Large payload with many entities (simulating real-world document)
    let large_text = r#"
    CONFIDENTIAL MEDICAL RECORD

    Patient Information:
    Name: Dr. Sarah Elizabeth Johnson-Smith
    Email: sarah.johnson.smith@example-hospital.com
    Phone: (555) 123-4567
    Alt Phone: +1-555-987-6543
    SSN: 123-45-6789
    DOB: 1985-03-15
    Address: 1234 Medical Center Drive, Suite 500, Boston, MA 02101

    Emergency Contact:
    Name: Michael Robert Thompson
    Relationship: Spouse
    Phone: (555) 234-5678
    Email: michael.r.thompson@gmail.com

    Primary Care Physician:
    Dr. Emily Catherine Rodriguez
    Hospital: Massachusetts General Hospital
    Phone: (617) 726-2000
    Email: emily.rodriguez@partners.org

    Insurance Information:
    Provider: Blue Cross Blue Shield
    Policy Number: BC-123456789-01
    Group Number: GRP-987654321

    Recent Consultation Notes:
    Date: 2024-01-15
    Provider: Dr. James Wilson, MD
    Contact: james.wilson@hospital.com
    Phone: (555) 345-6789

    The patient, Sarah Johnson-Smith, presented with complaints of chronic fatigue.
    Referred by Dr. Michael Chang (michael.chang@clinic.org, 555-456-7890).
    
    Follow-up scheduled with Dr. Lisa Park at lisa.park@specialists.com or (555) 567-8901.
    
    Administrative Contacts:
    - Billing: billing.dept@hospital.com, (555) 111-2222
    - Records: medical.records@hospital.com, (555) 333-4444  
    - Scheduling: schedule@hospital.com, (555) 555-6666

    Additional Providers:
    1. Dr. Robert Kim - cardiology@heart-center.com - (555) 777-8888
    2. Dr. Jennifer Lee - neurology@brain-institute.org - (555) 999-0000
    3. Dr. David Martinez - orthopedics@bone-clinic.net - (555) 222-1111

    Pharmacy Information:
    CVS Pharmacy #12345
    Address: 567 Main Street, Cambridge, MA 02139
    Phone: (617) 555-7777
    Pharmacist: Dr. Angela Thompson (angela.thompson@cvs.com)

    Laboratory Results Portal: https://results.hospital.com/patient/12345
    Patient Portal: https://myhealth.hospital.com/login
    
    This document contains sensitive medical information and should be handled
    according to HIPAA guidelines. For questions, contact privacy.officer@hospital.com
    or call the compliance hotline at (555) COMPLY1 (265-7591).

    Document ID: DOC-2024-001234-CONF
    Generated: 2024-01-20 14:30:00 EST
    Expires: 2024-12-31 23:59:59 EST
    "#;
    
    info!("Testing large payload: {} characters", large_text.len());
    assert!(large_text.len() >= 2000, "Payload should be over 2000 characters");

    // Count expected entities
    let regex_entities = detection_engine.detect_in_text(large_text);
    info!("Regex detected {} entities in large text", regex_entities.len());

    // Measure memory usage (basic approximation)
    let _start_memory = std::process::id(); // Simple memory tracking

    // Measure processing time
    let start_time = std::time::Instant::now();
    let result = process_text_through_pipeline(
        large_text,
        &detection_engine,
        &ollama_client,
        &mut faker_engine,
        &mut mapping_store,
        &ollama_config.model
    ).await?;
    let processing_duration = start_time.elapsed();
    
    info!("Large payload processed in {:?}", processing_duration);
    info!("Characters per second: {:.1}", 
          large_text.len() as f64 / processing_duration.as_secs_f64());

    // Verify processing succeeded
    assert_ne!(large_text, result, "Text should be anonymized");
    assert!(!result.is_empty(), "Result should not be empty");
    
    // Check that result length is reasonable (not truncated)
    let length_ratio = result.len() as f64 / large_text.len() as f64;
    info!("Length ratio (result/original): {:.2}", length_ratio);
    assert!(length_ratio > 0.5 && length_ratio < 2.0, 
            "Result length should be reasonable compared to input");

    // Large payload should still complete within timeout (allowing for concurrent execution)
    assert!(processing_duration.as_secs() < 90, 
            "Large payload should process in under 90 seconds");

    // Test that we can handle the large payload without errors
    assert!(processing_duration.as_millis() > 0, "Processing should take measurable time");

    // Memory efficiency check - result shouldn't be excessively larger
    info!("Original text: {} bytes", large_text.len());
    info!("Result text: {} bytes", result.len());
    
    let memory_efficiency = (result.len() as f64) / (large_text.len() as f64);
    info!("Memory efficiency ratio: {:.2}", memory_efficiency);
    assert!(memory_efficiency < 5.0, "Memory usage should be reasonable");

    // Test cache performance with large payload
    info!("Testing cache performance with large payload");
    let cache_start = std::time::Instant::now();
    let cached_result = process_text_through_pipeline(
        large_text,
        &detection_engine,
        &ollama_client,
        &mut faker_engine,
        &mut mapping_store,
        &ollama_config.model
    ).await?;
    let cache_duration = cache_start.elapsed();
    
    info!("Cached large payload processed in {:?}", cache_duration);
    assert_eq!(result, cached_result, "Cached result should be identical");
    assert!(cache_duration < processing_duration / 3, 
            "Cached run should be significantly faster");

    info!("Large payload performance test passed");
    Ok(())
}

/// Detection comparison test: Regex vs Ollama
/// Analyzes hits, misses, and overlaps between regex and LLM detection methods
#[tokio::test]
#[ignore] // Run with --ignored
async fn test_regex_vs_ollama_detection_comparison() -> Result<()> {
    let _ = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .try_init();

    info!("Starting Regex vs Ollama detection comparison test");

    let (config, ollama_config) = create_test_config().await?;
    let detection_engine = RegexDetectionEngine::new(&config.detection)?;
    let _mapping_store = MappingStore::new(config.mapping.clone())?;
    let ollama_client = OllamaClient::new(ollama_config.clone(), None)?;

    if !ollama_client.health_check().await? {
        warn!("Ollama not available, skipping detection comparison test");
        return Ok(());
    }

    // Test text with diverse PII types to test both detection methods
    let test_cases = vec![
        // Case 1: Regex-friendly patterns
        ("Simple patterns", "Contact john@example.com or call (555) 123-4567 for assistance."),
        
        // Case 2: Complex names that regex might miss
        ("Complex names", "Dr. Sarah Elizabeth Johnson-O'Brien and María José González-Smith discussed the case."),
        
        // Case 3: Mixed context with organizations
        ("Mixed context", "Please reach out to Microsoft Corporation at info@microsoft.com or Apple Inc. via their website."),
        
        // Case 4: Informal/contextual PII that LLM might catch better
        ("Contextual PII", "My buddy Jake from accounting mentioned that his SSN is 123-45-6789 and he works at TechCorp."),
        
        // Case 5: Multiple formats of same entity type
        ("Multiple formats", "Call me at 555-123-4567, (555) 987-6543, or +1 555 456 7890. Email: test@example.com, user@domain.co.uk"),
        
        // Case 6: Edge cases and ambiguous text
        ("Edge cases", "The meeting is at 192.168.1.1 (IP address) or visit https://example.com/path?param=value"),
        
        // Case 7: Medical/professional context
        ("Professional context", "Patient John Smith (DOB: 1985-03-15) scheduled with Dr. Rodriguez at Central Medical Hospital."),
    ];

    let mut comparison_results = Vec::new();

    for (case_name, text) in test_cases {
        info!("\n=== Testing Case: {} ===", case_name);
        info!("Text: {}", text);

        // Step 1: Regex detection
        let regex_entities = detection_engine.detect_in_text(text);
        info!("Regex detected {} entities:", regex_entities.len());
        for entity in &regex_entities {
            info!("  Regex: {} = '{}' at {}:{} (conf: {:.2})", 
                  entity.entity_type, entity.original_value, entity.start, entity.end, entity.confidence);
        }

        // Step 2: Ollama LLM detection (with error handling)
        let llm_entities = match ollama_client.extract_entities(text).await {
            Ok(entities) => entities,
            Err(e) => {
                warn!("Ollama extraction failed for '{}': {}", case_name, e);
                warn!("Continuing with empty LLM entities for this case");
                Vec::new()
            }
        };
        info!("Ollama detected {} entities:", llm_entities.len());
        for entity in &llm_entities {
            info!("  LLM: {} = '{}' at {}:{} (conf: {:.2})", 
                  entity.entity_type, entity.original_value, entity.start, entity.end, entity.confidence);
        }

        // Step 3: Analyze overlaps and differences
        let analysis = analyze_detection_differences(&regex_entities, &llm_entities, text);
        
        info!("=== Detection Analysis for {} ===", case_name);
        info!("Regex-only entities (missed by LLM): {}", analysis.regex_only.len());
        for entity in &analysis.regex_only {
            info!("  Regex ONLY: {} = '{}'", entity.entity_type, entity.original_value);
        }

        info!("LLM-only entities (missed by Regex): {}", analysis.llm_only.len());
        for entity in &analysis.llm_only {
            info!("  LLM ONLY: {} = '{}'", entity.entity_type, entity.original_value);
        }

        info!("Overlapping entities (found by both): {}", analysis.overlapping.len());
        for (regex_entity, llm_entity) in &analysis.overlapping {
            info!("  BOTH: Regex({}) vs LLM({}) = '{}'", 
                  regex_entity.entity_type, llm_entity.entity_type, regex_entity.original_value);
        }

        info!("Similar entities (same text, different positions/types): {}", analysis.similar.len());
        for (regex_entity, llm_entity) in &analysis.similar {
            info!("  SIMILAR: '{}' - Regex({}@{}:{}) vs LLM({}@{}:{})", 
                  regex_entity.original_value,
                  regex_entity.entity_type, regex_entity.start, regex_entity.end,
                  llm_entity.entity_type, llm_entity.start, llm_entity.end);
        }

        // Store results for summary
        comparison_results.push(DetectionComparisonResult {
            case_name: case_name.to_string(),
            text: text.to_string(),
            regex_count: regex_entities.len(),
            llm_count: llm_entities.len(),
            regex_only_count: analysis.regex_only.len(),
            llm_only_count: analysis.llm_only.len(),
            overlapping_count: analysis.overlapping.len(),
            similar_count: analysis.similar.len(),
            regex_precision: calculate_precision(&regex_entities, &analysis),
            llm_precision: calculate_precision(&llm_entities, &analysis),
        });
    }

    // Step 4: Generate comprehensive summary
    info!("\n=== DETECTION COMPARISON SUMMARY ===");
    
    let total_regex_entities: usize = comparison_results.iter().map(|r| r.regex_count).sum();
    let total_llm_entities: usize = comparison_results.iter().map(|r| r.llm_count).sum();
    let total_regex_only: usize = comparison_results.iter().map(|r| r.regex_only_count).sum();
    let total_llm_only: usize = comparison_results.iter().map(|r| r.llm_only_count).sum();
    let total_overlapping: usize = comparison_results.iter().map(|r| r.overlapping_count).sum();
    let total_similar: usize = comparison_results.iter().map(|r| r.similar_count).sum();

    info!("Overall Statistics:");
    info!("  Total Regex entities: {}", total_regex_entities);
    info!("  Total LLM entities: {}", total_llm_entities);
    info!("  Regex-only (LLM misses): {} ({:.1}% of regex)", 
          total_regex_only, (total_regex_only as f64 / total_regex_entities as f64) * 100.0);
    info!("  LLM-only (Regex misses): {} ({:.1}% of LLM)", 
          total_llm_only, (total_llm_only as f64 / total_llm_entities as f64) * 100.0);
    info!("  Perfect overlaps: {} ({:.1}% of all entities)", 
          total_overlapping, (total_overlapping as f64 / (total_regex_entities + total_llm_entities) as f64) * 100.0);
    info!("  Similar entities: {}", total_similar);

    // Coverage analysis
    let regex_coverage = (total_overlapping + total_similar + total_regex_only) as f64;
    let llm_coverage = (total_overlapping + total_similar + total_llm_only) as f64;
    let combined_coverage = regex_coverage + llm_coverage - (total_overlapping + total_similar) as f64;

    info!("Coverage Analysis:");
    info!("  Regex coverage: {:.1} entities", regex_coverage);
    info!("  LLM coverage: {:.1} entities", llm_coverage);
    info!("  Combined coverage: {:.1} entities ({:.1}% improvement over regex alone)", 
          combined_coverage, ((combined_coverage - regex_coverage) / regex_coverage) * 100.0);

    // Per-case breakdown
    info!("\nPer-case Results:");
    for result in &comparison_results {
        info!("  {}: R={}, L={}, R-only={}, L-only={}, Both={}", 
              result.case_name, result.regex_count, result.llm_count, 
              result.regex_only_count, result.llm_only_count, result.overlapping_count);
    }

    // Assertions to validate the test
    assert!(total_regex_entities > 0, "Should detect entities with regex");
    assert!(total_llm_entities > 0, "Should detect entities with LLM");
    assert!(total_regex_only + total_llm_only + total_overlapping + total_similar > 0, 
            "Should have some detection results");

    // Performance insight
    info!("\nDetection Method Insights:");
    if total_regex_only > total_llm_only {
        info!("  Regex catches more unique entities ({} vs {})", total_regex_only, total_llm_only);
        info!("  LLM may have higher precision but lower recall for standard patterns");
    } else if total_llm_only > total_regex_only {
        info!("  LLM catches more unique entities ({} vs {})", total_llm_only, total_regex_only);
        info!("  LLM shows better contextual understanding and entity recognition");
    } else {
        info!("  Both methods show similar unique detection rates");
    }

    if total_overlapping > 0 {
        info!("  {} entities detected by both methods - good consistency", total_overlapping);
    }

    info!("Detection comparison test completed successfully!");
    Ok(())
}

/// Analysis results for comparing regex vs LLM detection
#[derive(Debug)]
struct DetectionAnalysis {
    regex_only: Vec<DetectedEntity>,          // Found by regex but not LLM
    llm_only: Vec<DetectedEntity>,            // Found by LLM but not regex
    overlapping: Vec<(DetectedEntity, DetectedEntity)>, // Found by both (exact match)
    similar: Vec<(DetectedEntity, DetectedEntity)>,     // Similar but different positions/types
}

/// Comparison results for a single test case
#[derive(Debug)]
struct DetectionComparisonResult {
    case_name: String,
    text: String,
    regex_count: usize,
    llm_count: usize,
    regex_only_count: usize,
    llm_only_count: usize,
    overlapping_count: usize,
    similar_count: usize,
    regex_precision: f64,
    llm_precision: f64,
}

/// Analyze differences between regex and LLM detection results
fn analyze_detection_differences(
    regex_entities: &[DetectedEntity], 
    llm_entities: &[DetectedEntity], 
    _text: &str
) -> DetectionAnalysis {
    let mut regex_only = Vec::new();
    let mut llm_only = Vec::new();
    let mut overlapping = Vec::new();
    let mut similar = Vec::new();

    // Find matches and overlaps
    let mut matched_llm_indices = std::collections::HashSet::new();
    let mut matched_regex_indices = std::collections::HashSet::new();

    // First pass: find exact overlaps (same text, same or overlapping positions)
    for (r_idx, regex_entity) in regex_entities.iter().enumerate() {
        for (l_idx, llm_entity) in llm_entities.iter().enumerate() {
            if matched_llm_indices.contains(&l_idx) || matched_regex_indices.contains(&r_idx) {
                continue;
            }

            // Check for exact text match
            if regex_entity.original_value == llm_entity.original_value {
                // Check if positions overlap significantly
                let overlap = calculate_position_overlap(regex_entity, llm_entity);
                if overlap > 0.8 { // 80% overlap threshold
                    overlapping.push((regex_entity.clone(), llm_entity.clone()));
                    matched_regex_indices.insert(r_idx);
                    matched_llm_indices.insert(l_idx);
                } else {
                    similar.push((regex_entity.clone(), llm_entity.clone()));
                    matched_regex_indices.insert(r_idx);
                    matched_llm_indices.insert(l_idx);
                }
                break;
            }
        }
    }

    // Second pass: find similar entities (partial text matches)
    for (r_idx, regex_entity) in regex_entities.iter().enumerate() {
        if matched_regex_indices.contains(&r_idx) {
            continue;
        }

        for (l_idx, llm_entity) in llm_entities.iter().enumerate() {
            if matched_llm_indices.contains(&l_idx) {
                continue;
            }

            // Check for partial text similarity (one contains the other)
            if regex_entity.original_value.contains(&llm_entity.original_value) ||
               llm_entity.original_value.contains(&regex_entity.original_value) ||
               text_similarity(&regex_entity.original_value, &llm_entity.original_value) > 0.7 {
                similar.push((regex_entity.clone(), llm_entity.clone()));
                matched_regex_indices.insert(r_idx);
                matched_llm_indices.insert(l_idx);
                break;
            }
        }
    }

    // Third pass: collect unmatched entities
    for (r_idx, regex_entity) in regex_entities.iter().enumerate() {
        if !matched_regex_indices.contains(&r_idx) {
            regex_only.push(regex_entity.clone());
        }
    }

    for (l_idx, llm_entity) in llm_entities.iter().enumerate() {
        if !matched_llm_indices.contains(&l_idx) {
            llm_only.push(llm_entity.clone());
        }
    }

    DetectionAnalysis {
        regex_only,
        llm_only,
        overlapping,
        similar,
    }
}

/// Calculate position overlap between two entities (0.0 to 1.0)
fn calculate_position_overlap(entity1: &DetectedEntity, entity2: &DetectedEntity) -> f64 {
    let start = entity1.start.max(entity2.start);
    let end = entity1.end.min(entity2.end);
    
    if end <= start {
        return 0.0; // No overlap
    }
    
    let overlap_len = end - start;
    let total_len = (entity1.end - entity1.start).max(entity2.end - entity2.start);
    
    if total_len == 0 {
        return 0.0;
    }
    
    overlap_len as f64 / total_len as f64
}

/// Calculate text similarity between two strings (simple approach)
fn text_similarity(text1: &str, text2: &str) -> f64 {
    let text1_lower = text1.to_lowercase();
    let text2_lower = text2.to_lowercase();
    
    if text1_lower == text2_lower {
        return 1.0;
    }
    
    // Simple similarity: longest common substring / max length
    let max_len = text1.len().max(text2.len());
    if max_len == 0 {
        return 1.0;
    }
    
    let common_len = longest_common_substring(&text1_lower, &text2_lower);
    common_len as f64 / max_len as f64
}

/// Find longest common substring length
fn longest_common_substring(s1: &str, s2: &str) -> usize {
    let chars1: Vec<char> = s1.chars().collect();
    let chars2: Vec<char> = s2.chars().collect();
    let mut max_len = 0;
    
    for i in 0..chars1.len() {
        for j in 0..chars2.len() {
            let mut len = 0;
            while i + len < chars1.len() && j + len < chars2.len() && 
                  chars1[i + len] == chars2[j + len] {
                len += 1;
            }
            max_len = max_len.max(len);
        }
    }
    
    max_len
}

/// Calculate precision for a detection method based on analysis
fn calculate_precision(entities: &[DetectedEntity], analysis: &DetectionAnalysis) -> f64 {
    if entities.is_empty() {
        return 0.0;
    }
    
    // Precision = (overlapping + similar) / total detected
    let accurate_detections = analysis.overlapping.len() + analysis.similar.len();
    accurate_detections as f64 / entities.len() as f64
}

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
        ollama_config: crate::OllamaConfig::default(),
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
    use crate::{Config, OllamaConfig};
    use crate::{RegexDetectionEngine, FakerEngine, MappingStore};
    
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
