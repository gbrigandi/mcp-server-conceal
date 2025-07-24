//! Integrated MCP Privacy Proxy implementation

use anyhow::Result;
use serde_json::Value;
use std::collections::HashMap;
use std::path::PathBuf;
use tokio::io::{stdin, stdout, AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

use crate::config::{Config, DetectedEntity, AnonymizedEntity, DetectionMode};
use crate::detection::RegexDetectionEngine;
use crate::faker::FakerEngine;
use crate::mapping::MappingStore;
use crate::ollama::{OllamaClient, OllamaConfig};

#[derive(Debug, Clone)]
pub struct IntegratedProxyConfig {
    pub target_command: String,
    pub target_args: Vec<String>,
    pub target_env: HashMap<String, String>,
    pub target_cwd: Option<PathBuf>,
    pub config: Config,
    pub ollama_config: OllamaConfig,
}

pub struct IntegratedProxy {
    config: IntegratedProxyConfig,
    detection_engine: RegexDetectionEngine,
    faker_engine: FakerEngine,
    mapping_store: MappingStore,
    ollama_client: OllamaClient,
}

impl IntegratedProxy {
    pub fn new(config: IntegratedProxyConfig) -> Result<Self> {
        let detection_engine = RegexDetectionEngine::new(&config.config.detection)?;
        let faker_engine = FakerEngine::new(&config.config.faker);
        let mapping_store = MappingStore::new(config.config.mapping.clone())?;
        let ollama_client = OllamaClient::new(config.ollama_config.clone(), config.config.llm.as_ref().and_then(|llm| llm.prompt_template.as_ref()))?;

        Ok(Self {
            config,
            detection_engine,
            faker_engine,
            mapping_store,
            ollama_client,
        })
    }

    pub async fn run(&mut self) -> Result<()> {
        info!("Starting Integrated MCP Privacy Proxy");
        info!("  Regex patterns: {}", self.config.config.detection.patterns.len());
        info!("  Ollama enabled: {}", self.config.ollama_config.enabled);
        info!("  Database path: {}", self.config.config.mapping.database_path.display());

        let mut child = self.spawn_child_process().await?;
        let io_handles = self.setup_io_handles(&mut child)?;
        
        let (shutdown_tx, mut shutdown_rx) = mpsc::unbounded_channel();
        let tasks = self.spawn_processing_tasks(io_handles, shutdown_tx.clone()).await;

        shutdown_rx.recv().await;
        info!("Shutting down proxy");

        self.cleanup_tasks(tasks).await;
        self.print_final_stats();

        info!("Integrated MCP Privacy Proxy shut down");
        Ok(())
    }

    async fn spawn_child_process(&self) -> Result<Child> {
        info!(
            "Spawning child process: {} {:?}",
            self.config.target_command, self.config.target_args
        );

        let mut command = Command::new(&self.config.target_command);
        command
            .args(&self.config.target_args)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true);

        for (key, value) in &self.config.target_env {
            command.env(key, value);
            debug!("Setting env var: {}={}", key, value);
        }

        if let Some(ref cwd) = self.config.target_cwd {
            command.current_dir(cwd);
            debug!("Setting working directory: {}", cwd.display());
        }

        let child = command.spawn()
            .map_err(|e| anyhow::anyhow!("Failed to spawn child process '{}': {}", 
                                       self.config.target_command, e))?;

        info!("Child process started with PID: {:?}", child.id());
        Ok(child)
    }

    fn setup_io_handles(&self, child: &mut Child) -> Result<IoHandles> {
        let child_stdin = child.stdin.take()
            .ok_or_else(|| anyhow::anyhow!("Failed to get child stdin"))?;
        let child_stdout = child.stdout.take()
            .ok_or_else(|| anyhow::anyhow!("Failed to get child stdout"))?;
        let child_stderr = child.stderr.take()
            .ok_or_else(|| anyhow::anyhow!("Failed to get child stderr"))?;

        Ok(IoHandles {
            child_stdin,
            child_stdout,
            child_stderr,
            our_stdin: stdin(),
            our_stdout: stdout(),
        })
    }

    async fn spawn_processing_tasks(&self, handles: IoHandles, shutdown_tx: mpsc::UnboundedSender<()>) -> ProxyTasks {
        let stdin_task = self.spawn_stdin_task(handles.our_stdin, handles.child_stdin, shutdown_tx.clone()).await;
        let stdout_task = self.spawn_stdout_task(handles.child_stdout, handles.our_stdout, shutdown_tx.clone()).await;
        let stderr_task = spawn_stderr_task(handles.child_stderr, shutdown_tx.clone());
        // Temporarily disable child monitor task that was causing immediate shutdown
        let child_task = tokio::spawn(async move {
            // Do nothing - let the stdin/stdout tasks handle shutdown
            std::future::pending::<()>().await
        });

        ProxyTasks {
            stdin_task,
            stdout_task, 
            stderr_task,
            child_task,
        }
    }

    async fn spawn_stdin_task(&self, our_stdin: tokio::io::Stdin, mut child_stdin: tokio::process::ChildStdin, shutdown_tx: mpsc::UnboundedSender<()>) -> tokio::task::JoinHandle<()> {
        let mut detection_engine = self.detection_engine.clone();
        let mut faker_engine = self.faker_engine.clone();
        let mapping_config = self.config.config.mapping.clone();
        let ollama_client = self.ollama_client.clone();
        let ollama_config = self.config.ollama_config.clone();
        let detection_mode = self.config.config.detection.mode.clone();

        tokio::spawn(async move {
            let mut mapping_store = match MappingStore::new(mapping_config) {
                Ok(store) => store,
                Err(e) => {
                    error!("Failed to create mapping store in stdin task: {}", e);
                    shutdown_tx.send(()).ok();
                    return;
                }
            };

            if let Err(e) = process_stdin_loop(
                our_stdin, 
                &mut child_stdin,
                &mut detection_engine,
                &ollama_client,
                &mut faker_engine,
                &mut mapping_store,
                &ollama_config.model,
                &detection_mode,
                &shutdown_tx
            ).await {
                error!("Stdin processing failed: {}", e);
            }
        })
    }

    async fn spawn_stdout_task(&self, child_stdout: tokio::process::ChildStdout, mut our_stdout: tokio::io::Stdout, shutdown_tx: mpsc::UnboundedSender<()>) -> tokio::task::JoinHandle<()> {
        let mut detection_engine = self.detection_engine.clone();
        let mut faker_engine = self.faker_engine.clone();
        let mapping_config = self.config.config.mapping.clone();
        let ollama_client = self.ollama_client.clone();
        let ollama_config = self.config.ollama_config.clone();
        let detection_mode = self.config.config.detection.mode.clone();

        tokio::spawn(async move {
            let mut mapping_store = match MappingStore::new(mapping_config) {
                Ok(store) => store,
                Err(e) => {
                    error!("Failed to create mapping store in stdout task: {}", e);
                    shutdown_tx.send(()).ok();
                    return;
                }
            };

            if let Err(e) = process_stdout_loop(
                child_stdout,
                &mut our_stdout,
                &mut detection_engine,
                &ollama_client,
                &mut faker_engine,
                &mut mapping_store,
                &ollama_config.model,
                &detection_mode,
                &shutdown_tx
            ).await {
                error!("Stdout processing failed: {}", e);
            }
        })
    }

    async fn cleanup_tasks(&self, tasks: ProxyTasks) {
        tasks.stdin_task.abort();
        tasks.stdout_task.abort();
        tasks.stderr_task.abort();
        tasks.child_task.abort();

        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    }

    fn print_final_stats(&self) {
        match self.mapping_store.get_statistics() {
            Ok(stats) => {
                info!("Final processing statistics:");
                info!("  Total mappings created: {}", stats.total_mappings);
                info!("  Cache entries: {}", stats.total_cache_entries);
                info!("  Entity types processed: {:?}", stats.mappings_by_type);
            }
            Err(e) => warn!("Failed to get final statistics: {}", e),
        }
    }
}

struct IoHandles {
    child_stdin: tokio::process::ChildStdin,
    child_stdout: tokio::process::ChildStdout,
    child_stderr: tokio::process::ChildStderr,
    our_stdin: tokio::io::Stdin,
    our_stdout: tokio::io::Stdout,
}

struct ProxyTasks {
    stdin_task: tokio::task::JoinHandle<()>,
    stdout_task: tokio::task::JoinHandle<()>,
    stderr_task: tokio::task::JoinHandle<()>,
    child_task: tokio::task::JoinHandle<()>,
}

async fn process_stdin_loop(
    our_stdin: tokio::io::Stdin,
    child_stdin: &mut tokio::process::ChildStdin,
    detection_engine: &mut RegexDetectionEngine,
    ollama_client: &OllamaClient,
    faker_engine: &mut FakerEngine,
    mapping_store: &mut MappingStore,
    model_name: &str,
    detection_mode: &DetectionMode,
    shutdown_tx: &mpsc::UnboundedSender<()>,
) -> Result<()> {
    let mut reader = BufReader::new(our_stdin);
    let mut line = String::new();

    loop {
        line.clear();
        match reader.read_line(&mut line).await {
            Ok(0) => {
                info!("EOF on stdin, shutting down");
                shutdown_tx.send(()).ok();
                break;
            }
            Ok(_) => {
                if let Err(e) = process_and_forward_line(
                    &line,
                    child_stdin,
                    detection_engine,
                    ollama_client,
                    faker_engine,
                    mapping_store,
                    model_name,
                    detection_mode,
                    "request"
                ).await {
                    error!("Failed to process stdin line: {}", e);
                    shutdown_tx.send(()).ok();
                    break;
                }
            }
            Err(e) => {
                error!("Failed to read from stdin: {}", e);
                shutdown_tx.send(()).ok();
                break;
            }
        }
    }
    Ok(())
}

async fn process_stdout_loop(
    child_stdout: tokio::process::ChildStdout,
    our_stdout: &mut tokio::io::Stdout,
    detection_engine: &mut RegexDetectionEngine,
    ollama_client: &OllamaClient,
    faker_engine: &mut FakerEngine,
    mapping_store: &mut MappingStore,
    model_name: &str,
    detection_mode: &DetectionMode,
    shutdown_tx: &mpsc::UnboundedSender<()>,
) -> Result<()> {
    let mut reader = BufReader::new(child_stdout);
    let mut line = String::new();

    loop {
        line.clear();
        match reader.read_line(&mut line).await {
            Ok(0) => {
                info!("EOF on child stdout");
                shutdown_tx.send(()).ok();
                break;
            }
            Ok(_) => {
                if let Err(e) = process_and_forward_line(
                    &line,
                    our_stdout,
                    detection_engine,
                    ollama_client,
                    faker_engine,
                    mapping_store,
                    model_name,
                    detection_mode,
                    "response"
                ).await {
                    error!("Failed to process stdout line: {}", e);
                    shutdown_tx.send(()).ok();
                    break;
                }
            }
            Err(e) => {
                error!("Failed to read from child stdout: {}", e);
                shutdown_tx.send(()).ok();
                break;
            }
        }
    }
    Ok(())
}

async fn process_and_forward_line<W: AsyncWriteExt + Unpin>(
    line: &str,
    writer: &mut W,
    detection_engine: &mut RegexDetectionEngine,
    ollama_client: &OllamaClient,
    faker_engine: &mut FakerEngine,
    mapping_store: &mut MappingStore,
    model_name: &str,
    detection_mode: &DetectionMode,
    direction: &str,
) -> Result<()> {
    let original_line = line.trim();
    debug!("Processing {}: {}", direction, original_line);

    match process_request_with_pii_detection(
        original_line,
        detection_engine,
        ollama_client,
        faker_engine,
        mapping_store,
        model_name,
        detection_mode,
    ).await {
        Ok(processed_line) => {
            if processed_line != original_line {
                info!("PII detected and anonymized in {}", direction);
                debug!("Original: {}", original_line);
                debug!("Processed: {}", processed_line);
            }
            
            writer.write_all((processed_line + "\n").as_bytes()).await?;
            writer.flush().await?;
        }
        Err(e) => {
            warn!("Error processing {} for PII, forwarding original: {}", direction, e);
            writer.write_all(line.as_bytes()).await?;
            writer.flush().await?;
        }
    }
    Ok(())
}

fn spawn_stderr_task(child_stderr: tokio::process::ChildStderr, _shutdown_tx: mpsc::UnboundedSender<()>) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut reader = BufReader::new(child_stderr);
        let mut line = String::new();

        loop {
            line.clear();
            match reader.read_line(&mut line).await {
                Ok(0) => {
                    debug!("EOF on child stderr");
                    break;
                }
                Ok(_) => {
                    warn!("Child stderr: {}", line.trim());
                }
                Err(e) => {
                    error!("Failed to read from child stderr: {}", e);
                    break;
                }
            }
        }
    })
}


async fn process_request_with_pii_detection(
    line: &str,
    detection_engine: &mut RegexDetectionEngine,
    ollama_client: &OllamaClient,
    faker_engine: &mut FakerEngine,
    mapping_store: &mut MappingStore,
    model_name: &str,
    detection_mode: &DetectionMode,
) -> Result<String> {
    let json_value: Value = serde_json::from_str(line)?;
    
    // Check if this is a JSON-RPC/MCP protocol message - if so, skip PII processing
    if is_jsonrpc_protocol_message(&json_value) {
        debug!("Skipping PII processing for JSON-RPC/MCP protocol message");
        return Ok(line.to_string());
    }
    
    let mut json_value = json_value;
    let any_changes = process_json_for_pii(
        &mut json_value, 
        detection_engine, 
        ollama_client, 
        faker_engine, 
        mapping_store, 
        model_name,
        detection_mode
    ).await.unwrap_or(false);
    
    if any_changes {
        serde_json::to_string(&json_value)
            .map_err(|e| anyhow::anyhow!("Failed to serialize modified JSON: {}", e))
    } else {
        Ok(line.to_string())
    }
}

fn is_jsonrpc_protocol_message(json_value: &Value) -> bool {
    if let Some(obj) = json_value.as_object() {
        // MCP protocol control messages - skip PII processing
        if obj.contains_key("protocolVersion") ||
           obj.contains_key("capabilities") ||
           obj.contains_key("serverInfo") ||
           obj.contains_key("clientInfo") {
            return true;
        }
        
        // JSON-RPC requests - skip PII processing
        if obj.contains_key("method") && obj.contains_key("id") {
            return true;
        }
        
        // JSON-RPC error responses - skip PII processing
        if obj.contains_key("error") && obj.contains_key("id") {
            return true;
        }
        
        // JSON-RPC responses with results - check if they contain user data
        if obj.contains_key("result") && obj.contains_key("id") {
            // If the result contains a "content" field, this is likely tool response data that should be processed
            if let Some(result) = obj.get("result") {
                if let Some(result_obj) = result.as_object() {
                    if result_obj.contains_key("content") {
                        return false; // Process this for PII - it contains user data
                    }
                }
            }
            // Otherwise, it's a protocol control response (initialize, tools/list, etc.)
            return true;
        }
        
        // Non-JSON-RPC messages should be processed
        false
    } else {
        false
    }
}

fn process_json_for_pii<'a>(
    value: &'a mut Value,
    detection_engine: &'a mut RegexDetectionEngine,
    ollama_client: &'a OllamaClient,
    faker_engine: &'a mut FakerEngine,
    mapping_store: &'a mut MappingStore,
    model_name: &'a str,
    detection_mode: &'a DetectionMode,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<bool>> + Send + 'a>> {
    Box::pin(async move {
        let mut any_changes = false;
        
        match value {
            Value::String(text) => {
                // Only bother with non-trivial strings
                if text.trim().len() > 3 {
                    if let Ok(processed_text) = process_text_through_pipeline(
                        text,
                        detection_engine,
                        ollama_client,
                        faker_engine,
                        mapping_store,
                        model_name,
                        detection_mode,
                    ).await {
                        if processed_text != *text {
                            *text = processed_text;
                            any_changes = true;
                        }
                    }
                }
            }
            Value::Array(arr) => {
                for item in arr.iter_mut() {
                    if process_json_for_pii(item, detection_engine, ollama_client, faker_engine, mapping_store, model_name, detection_mode).await? {
                        any_changes = true;
                    }
                }
            }
            Value::Object(obj) => {
                for (_, val) in obj.iter_mut() {
                    if process_json_for_pii(val, detection_engine, ollama_client, faker_engine, mapping_store, model_name, detection_mode).await? {
                        any_changes = true;
                    }
                }
            }
            _ => {}
        }
        
        Ok(any_changes)
    })
}

async fn process_text_through_pipeline(
    text: &str,
    detection_engine: &mut RegexDetectionEngine,
    ollama_client: &OllamaClient,
    faker_engine: &mut FakerEngine,
    mapping_store: &mut MappingStore,
    model_name: &str,
    detection_mode: &DetectionMode,
) -> Result<String> {
    let combined_entities = match detection_mode {
        DetectionMode::Regex => {
            // Regex-only detection
            detection_engine.detect_in_text(text)
        }
        DetectionMode::Llm => {
            // LLM-only detection
            let llm_entities = get_llm_entities(text, ollama_client, mapping_store, model_name).await?;
            llm_entities
        }
        DetectionMode::RegexLlm => {
            // Hybrid approach: regex first, then LLM
            let regex_entities = detection_engine.detect_in_text(text);
            let llm_entities = get_llm_entities(text, ollama_client, mapping_store, model_name).await?;
            combine_entities(regex_entities, llm_entities)
        }
    };
    
    if combined_entities.is_empty() {
        return Ok(text.to_string());
    }
    
    let anonymized_entities = create_anonymized_entities(combined_entities, faker_engine, mapping_store).await?;
    apply_replacements(text, &anonymized_entities)
}

async fn get_llm_entities(
    text: &str,
    ollama_client: &OllamaClient,
    mapping_store: &mut MappingStore,
    model_name: &str,
) -> Result<Vec<DetectedEntity>> {
    // Check cache first
    if let Some(cached) = mapping_store.get_llm_cache(text, model_name)? {
        return Ok(cached);
    }
    
    // Try LLM if available
    if ollama_client.health_check().await.unwrap_or(false) {
        match ollama_client.extract_entities(text).await {
            Ok(entities) => {
                mapping_store.store_llm_cache(text, &entities, model_name)?;
                Ok(entities)
            }
            Err(e) => {
                debug!("Ollama extraction failed, using regex-only: {}", e);
                Ok(Vec::new())
            }
        }
    } else {
        debug!("Ollama not available, using regex-only detection");
        Ok(Vec::new())
    }
}

async fn create_anonymized_entities(
    entities: Vec<DetectedEntity>,
    faker_engine: &mut FakerEngine,
    mapping_store: &mut MappingStore,
) -> Result<Vec<AnonymizedEntity>> {
    let mut anonymized_entities = Vec::new();
    
    for entity in entities {
        let anonymized = if let Some(existing_fake) = mapping_store.get_mapping(&entity.entity_type, &entity.original_value)? {
            AnonymizedEntity {
                entity_type: entity.entity_type,
                original_value: entity.original_value,
                fake_value: existing_fake,
                mapping_id: format!("existing-{}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()),
            }
        } else {
            let anonymized = faker_engine.anonymize_entity(&entity)?;
            mapping_store.store_mapping(&anonymized)?;
            anonymized
        };
        anonymized_entities.push(anonymized);
    }
    
    Ok(anonymized_entities)
}

// Prefer deterministic deduplication over complex overlap detection
fn combine_entities(regex_entities: Vec<DetectedEntity>, llm_entities: Vec<DetectedEntity>) -> Vec<DetectedEntity> {
    let mut combined = HashMap::new();
    
    // Add regex entities first (lower priority)
    for entity in regex_entities {
        let key = format!("{}:{}:{}", entity.entity_type, entity.start, entity.end);
        combined.insert(key, entity);
    }
    
    // LLM entities override regex ones
    for entity in llm_entities {
        let key = format!("{}:{}:{}", entity.entity_type, entity.start, entity.end);
        combined.insert(key, entity);
    }
    
    combined.into_values().collect()
}

// Simple text replacement - good enough for most cases
fn apply_replacements(text: &str, entities: &[AnonymizedEntity]) -> Result<String> {
    let mut result = text.to_string();
    
    // Sort by position to avoid messing up indices during replacement
    let mut sorted_entities: Vec<_> = entities.iter().collect();
    sorted_entities.sort_by_key(|e| text.find(&e.original_value).unwrap_or(0));
    
    for entity in sorted_entities {
        result = result.replace(&entity.original_value, &entity.fake_value);
    }
    
    Ok(result)
}