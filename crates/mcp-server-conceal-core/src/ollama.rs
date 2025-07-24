//! Ollama LLM integration for Named Entity Recognition
//!
//! This module provides integration with Ollama for intelligent PII detection using
//! Large Language Models, with support for health checks and response parsing.

use anyhow::Result;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tracing::{debug, error, info, warn};
use crate::config::DetectedEntity;
use crate::prompt_loader::PromptLoader;

#[derive(Debug, Clone)]
pub struct OllamaConfig {
    pub endpoint: String,
    pub model: String,
    pub timeout_seconds: u64,
    pub enabled: bool,
}

impl Default for OllamaConfig {
    fn default() -> Self {
        Self {
            endpoint: "http://localhost:11434".to_string(),
            model: "llama3.2:3b".to_string(),
            timeout_seconds: 30,
            enabled: false,
        }
    }
}

#[derive(Debug, Serialize)]
struct OllamaRequest {
    model: String,
    prompt: String,
    stream: bool,
    options: OllamaOptions,
}

#[derive(Debug, Serialize)]
struct OllamaOptions {
    temperature: f32,
    top_p: f32,
    max_tokens: u32,
}

#[derive(Debug, Deserialize)]
struct OllamaResponse {
    response: String,
    done: bool,
}

#[derive(Debug, Deserialize)]
pub struct LlmResponse {
    pub entities: Vec<LlmDetectedEntity>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LlmDetectedEntity {
    #[serde(rename = "type")]
    pub entity_type: String,
    pub value: String,
    #[serde(default)]
    pub start: usize,
    #[serde(default)]
    pub end: usize,
    #[serde(default = "default_confidence")]
    pub confidence: f64,
}

fn default_confidence() -> f64 {
    0.8
}

#[derive(Clone)]
pub struct OllamaClient {
    client: Client,
    config: OllamaConfig,
    prompt_loader: PromptLoader,
    prompt_template: String,
}

impl OllamaClient {
    pub fn new(config: OllamaConfig, prompt_template: Option<&String>) -> Result<Self> {
        let client = Client::builder()
            .timeout(Duration::from_secs(config.timeout_seconds))
            .build()
            .expect("Failed to create HTTP client");

        let prompt_loader = PromptLoader::new()?;
        let template = prompt_loader.load_prompt(prompt_template)?;

        Ok(Self { 
            client, 
            config, 
            prompt_loader,
            prompt_template: template,
        })
    }

    pub async fn extract_entities(&self, text: &str) -> Result<Vec<DetectedEntity>> {
        if !self.config.enabled {
            debug!("Ollama client is disabled, returning empty entities");
            return Ok(vec![]);
        }

        debug!("Sending text to Ollama for LLM detection: {} characters", text.len());

        let prompt = self.prompt_loader.format_prompt(&self.prompt_template, text);
        let response = self.call_ollama(&prompt).await?;
        
        self.parse_llm_response(&response, text)
    }

    async fn call_ollama(&self, prompt: &str) -> Result<String> {
        let request = OllamaRequest {
            model: self.config.model.clone(),
            prompt: prompt.to_string(),
            stream: false,
            options: OllamaOptions {
                temperature: 0.0,  // Set to 0 for deterministic JSON output
                top_p: 0.1,        // Very low for strict adherence to format
                max_tokens: 500,   // Lower to prevent rambling
            },
        };

        debug!("Making request to Ollama: {}/api/generate", self.config.endpoint);
        
        let response = self.client
            .post(&format!("{}/api/generate", self.config.endpoint))
            .json(&request)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            error!("Ollama request failed with status {}: {}", status, error_text);
            return Err(anyhow::anyhow!("Ollama request failed: {} - {}", status, error_text));
        }

        let ollama_response: OllamaResponse = response.json().await?;
        
        if !ollama_response.done {
            warn!("Received incomplete response from Ollama");
        }

        debug!("Received response from Ollama: {} characters", ollama_response.response.len());
        Ok(ollama_response.response)
    }


    fn parse_llm_response(&self, response: &str, original_text: &str) -> Result<Vec<DetectedEntity>> {
        // Try to extract JSON from the response
        let json_str = self.extract_json_from_response(response)?;
        
        debug!("Parsing JSON response: {}", json_str);
        
        let llm_response: LlmResponse = serde_json::from_str(&json_str)
            .map_err(|e| anyhow::anyhow!("Failed to parse LLM JSON response: {}", e))?;

        let mut entities = Vec::new();
        
        for llm_entity in llm_response.entities {
            let (start, end) = if llm_entity.start == 0 && llm_entity.end == 0 {
                if let Some((found_start, found_end)) = self.find_entity_position(original_text, &llm_entity.value) {
                    (found_start, found_end)
                } else {
                    warn!("Could not find entity '{}' in text", llm_entity.value);
                    continue;
                }
            } else {
                if llm_entity.start >= llm_entity.end || llm_entity.end > original_text.len() {
                    warn!("Invalid entity positions for '{}': {}-{}", 
                          llm_entity.value, llm_entity.start, llm_entity.end);
                    if let Some((found_start, found_end)) = self.find_entity_position(original_text, &llm_entity.value) {
                        (found_start, found_end)
                    } else {
                        continue;
                    }
                } else {
                    let actual_text = &original_text[llm_entity.start..llm_entity.end];
                    if actual_text != llm_entity.value {
                        warn!("Entity value mismatch: expected '{}', found '{}'", 
                              llm_entity.value, actual_text);
                            if let Some((found_start, found_end)) = self.find_entity_position(original_text, &llm_entity.value) {
                            (found_start, found_end)
                        } else {
                            continue;
                        }
                    } else {
                        (llm_entity.start, llm_entity.end)
                    }
                }
            };

            entities.push(DetectedEntity {
                entity_type: llm_entity.entity_type,
                original_value: llm_entity.value,
                start,
                end,
                confidence: llm_entity.confidence,
            });
        }

        info!("Ollama extracted {} entities", entities.len());
        Ok(entities)
    }

    fn extract_json_from_response(&self, response: &str) -> Result<String> {
        // First, fix double braces that Ollama might return due to template parsing
        let fixed_response = response.replace("{{", "{").replace("}}", "}");
        
        // Try to find and extract the first complete JSON object
        if let Some(start) = fixed_response.find('{') {
            let mut brace_count = 0;
            let mut end_pos = start;
            let chars: Vec<char> = fixed_response.chars().collect();
            
            for (i, &ch) in chars.iter().enumerate().skip(start) {
                match ch {
                    '{' => brace_count += 1,
                    '}' => {
                        brace_count -= 1;
                        if brace_count == 0 {
                            end_pos = i;
                            break;
                        }
                    }
                    _ => {}
                }
            }
            
            if brace_count == 0 && end_pos > start {
                let json_str = fixed_response[start..=end_pos].to_string();
                // Validate it's actually valid JSON by trying to parse it
                if serde_json::from_str::<serde_json::Value>(&json_str).is_ok() {
                    return Ok(json_str);
                }
            }
        }

        // Fallback: if the entire response looks like JSON
        let trimmed = fixed_response.trim();
        if trimmed.starts_with('{') && trimmed.ends_with('}') {
            if serde_json::from_str::<serde_json::Value>(trimmed).is_ok() {
                return Ok(trimmed.to_string());
            }
        }

        Err(anyhow::anyhow!("No valid JSON found in Ollama response: {}", response))
    }

    fn find_entity_position(&self, text: &str, entity_value: &str) -> Option<(usize, usize)> {
        if let Some(start) = text.find(entity_value) {
            Some((start, start + entity_value.len()))
        } else {
            None
        }
    }

    pub async fn health_check(&self) -> Result<bool> {
        if !self.config.enabled {
            return Ok(false);
        }

        debug!("Performing Ollama health check");
        
        let response = self.client
            .get(&format!("{}/api/tags", self.config.endpoint))
            .send()
            .await?;

        let is_healthy = response.status().is_success();
        
        if is_healthy {
            info!("Ollama health check passed");
        } else {
            warn!("Ollama health check failed: {}", response.status());
        }

        Ok(is_healthy)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_config() -> OllamaConfig {
        OllamaConfig {
            endpoint: "http://localhost:11434".to_string(),
            model: "llama3.2:3b".to_string(),
            timeout_seconds: 30,
            enabled: true,
        }
    }

    #[test]
    fn test_ollama_config_default() {
        let config = OllamaConfig::default();
        
        assert_eq!(config.endpoint, "http://localhost:11434");
        assert_eq!(config.model, "llama3.2:3b");
        assert_eq!(config.timeout_seconds, 30);
        assert!(!config.enabled);
    }

    #[test]
    fn test_ollama_client_creation() {
        let config = create_test_config();
        let client = OllamaClient::new(config.clone(), None).unwrap();
        
        assert_eq!(client.config.model, "llama3.2:3b");
        assert_eq!(client.config.endpoint, "http://localhost:11434");
    }

    #[test]
    fn test_create_llm_prompt() {
        let config = create_test_config();
        let client = OllamaClient::new(config, None).unwrap();
        
        let text = "Contact Sarah Johnson at sarah@company.com";
        let prompt = client.prompt_loader.format_prompt(&client.prompt_template, text);
        
        assert!(prompt.contains(text));
        assert!(prompt.contains("person_name"));
        assert!(prompt.contains("JSON"));
        assert!(prompt.contains("entities"));
    }

    #[test]
    fn test_extract_json_from_response() {
        let config = create_test_config();
        let client = OllamaClient::new(config, None).unwrap();
        
        // Test with JSON embedded in text
        let response1 = r#"Here is the JSON: {"entities": [{"type": "person_name", "value": "John", "start": 0, "end": 4, "confidence": 0.9}]} End of response."#;
        let json1 = client.extract_json_from_response(response1).unwrap();
        assert!(json1.starts_with('{'));
        assert!(json1.ends_with('}'));
        assert!(serde_json::from_str::<serde_json::Value>(&json1).is_ok());
        
        // Test with pure JSON
        let response2 = r#"{"entities": []}"#;
        let json2 = client.extract_json_from_response(response2).unwrap();
        assert_eq!(json2, response2);
        
        // Test with multiple JSON objects (should extract only the first)
        let response3 = r#"{"entities": [{"type": "person_name", "value": "John", "start": 0, "end": 4, "confidence": 0.9}]}

{"entities": []}"#;
        let json3 = client.extract_json_from_response(response3).unwrap();
        assert!(json3.starts_with('{'));
        assert!(json3.ends_with('}'));
        assert!(serde_json::from_str::<serde_json::Value>(&json3).is_ok());
        // Should only contain the first JSON object
        assert!(json3.contains("John"));
        assert!(!json3.contains("}\n\n{"));
        
        // Test with double braces (Ollama template format)
        let response4 = r#"{{"entities": [{{"type": "person_name", "value": "Sarah Johnson", "start": 0, "end": 15, "confidence": 0.9}}]}}"#;
        let json4 = client.extract_json_from_response(response4).unwrap();
        assert!(json4.starts_with('{'));
        assert!(json4.ends_with('}'));
        assert!(serde_json::from_str::<serde_json::Value>(&json4).is_ok());
        assert!(json4.contains("Sarah Johnson"));
        // Should not contain double braces after processing
        assert!(!json4.contains("{{"));
        assert!(!json4.contains("}}"));
        
        // Test with no JSON
        let response5 = "No JSON here";
        assert!(client.extract_json_from_response(response5).is_err());
    }

    #[test]
    fn test_find_entity_position() {
        let config = create_test_config();
        let client = OllamaClient::new(config, None).unwrap();
        
        let text = "Contact Sarah Johnson at sarah@company.com";
        
        let pos = client.find_entity_position(text, "Sarah Johnson");
        assert_eq!(pos, Some((8, 21)));
        
        let pos2 = client.find_entity_position(text, "Not Found");
        assert_eq!(pos2, None);
    }

    #[test]
    fn test_parse_valid_llm_response() {
        let config = create_test_config();
        let client = OllamaClient::new(config, None).unwrap();
        
        let response = r#"{"entities": [{"type": "person_name", "value": "Sarah", "start": 8, "end": 13, "confidence": 0.95}]}"#;
        let original_text = "Contact Sarah Johnson";
        
        let entities = client.parse_llm_response(response, original_text).unwrap();
        
        assert_eq!(entities.len(), 1);
        assert_eq!(entities[0].entity_type, "person_name");
        assert_eq!(entities[0].original_value, "Sarah");
        assert_eq!(entities[0].start, 8);
        assert_eq!(entities[0].end, 13);
        assert_eq!(entities[0].confidence, 0.95);
    }

    #[test]
    fn test_parse_invalid_position_response() {
        let config = create_test_config();
        let client = OllamaClient::new(config, None).unwrap();
        
        let response = r#"{"entities": [{"type": "person_name", "value": "Sarah", "start": 8, "end": 13, "confidence": 0.95}]}"#;
        let original_text = "Contact John Johnson";
        
        let entities = client.parse_llm_response(response, original_text);
        
        assert!(entities.is_ok());
        let entities = entities.unwrap();
        assert!(entities.is_empty() || entities[0].original_value == "Sarah");
    }

    #[test]
    fn test_disabled_client() {
        let mut config = create_test_config();
        config.enabled = false;
        
        let client = OllamaClient::new(config, None).unwrap();
        
        tokio_test::block_on(async {
            let entities = client.extract_entities("Some text").await.unwrap();
            assert!(entities.is_empty());
        });
    }
}