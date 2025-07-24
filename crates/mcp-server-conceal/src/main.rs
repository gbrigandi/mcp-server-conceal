//! MCP Server Conceal command-line interface

use anyhow::Result;
use clap::Parser;
use std::collections::HashMap;
use std::path::PathBuf;
use tracing::{info, warn};

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
pub struct Args {
    #[arg(long, help = "Command to execute for the target MCP server")]
    pub target_command: String,

    #[arg(long, help = "Arguments for the target MCP server (space-separated)")]
    pub target_args: Option<String>,

    #[arg(long, action = clap::ArgAction::Append, help = "Environment variables for the target server (KEY=VALUE)")]
    pub target_env: Vec<String>,

    #[arg(long, help = "Working directory for the target MCP server")]
    pub target_cwd: Option<PathBuf>,

    #[arg(long, default_value = "info", help = "Log level (error, warn, info, debug, trace)")]
    pub log_level: String,

    #[arg(long, help = "Path to configuration file")]
    pub config: Option<PathBuf>,

    #[arg(long, help = "Keep existing database mappings (by default, database is cleared on each run)")]
    pub keep_database: bool,
}

impl Args {
    pub fn parse_target_args(&self) -> Vec<String> {
        self.target_args.as_ref()
            .and_then(|args| shell_words::split(args).ok())
            .unwrap_or_else(|| {
                if let Some(ref args_str) = self.target_args {
                    warn!("Failed to parse target args '{}'. Using as single argument.", args_str);
                    vec![args_str.clone()]
                } else {
                    vec![]
                }
            })
    }

    pub fn parse_target_env(&self) -> Result<HashMap<String, String>> {
        self.target_env.iter()
            .try_fold(HashMap::new(), |mut acc, env_var| {
                if let Some((key, value)) = env_var.split_once('=') {
                    acc.insert(key.to_string(), value.to_string());
                    Ok(acc)
                } else {
                    Err(anyhow::anyhow!("Invalid environment variable format: '{}'. Expected KEY=VALUE", env_var))
                }
            })
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    
    let log_level = args.log_level.parse::<tracing::Level>()
        .unwrap_or_else(|_| {
            eprintln!("Invalid log level '{}', defaulting to 'info'", args.log_level);
            tracing::Level::INFO
        });
    
    tracing_subscriber::fmt()
        .with_max_level(log_level)
        .with_target(false)
        .with_writer(std::io::stderr)
        .init();

    info!("Starting mcp-server-conceal proxy");
    info!("Target command: {}", args.target_command);
    info!("Target args: {:?}", args.parse_target_args());
    
    let target_env = args.parse_target_env()?;
    info!("Target environment variables: {} entries", target_env.len());
    
    if let Some(ref cwd) = args.target_cwd {
        info!("Target working directory: {}", cwd.display());
    }

    let config = match args.config.as_ref() {
        Some(config_path) => {
            info!("Loading configuration from: {}", config_path.display());
            mcp_server_conceal_core::Config::from_file(config_path)?
        }
        None => {
            // Try to load from default location, fallback to default config
            match mcp_server_conceal_core::Config::get_default_config_path() {
                Ok(default_path) if default_path.exists() => {
                    info!("Loading configuration from default location: {}", default_path.display());
                    mcp_server_conceal_core::Config::from_file(&default_path)?
                }
                Ok(default_path) => {
                    info!("Creating default configuration at: {}", default_path.display());
                    let mut config = mcp_server_conceal_core::Config::default();
                    config.resolve_paths()?;
                    config.to_file(&default_path)?;
                    config
                }
                Err(_) => {
                    info!("Using default configuration (could not determine config directory)");
                    let mut config = mcp_server_conceal_core::Config::default();
                    config.resolve_paths()?;
                    config
                }
            }
        }
    };

    config.validate()?;
    info!("Configuration validated successfully");

    // Remove database by default unless --keep-database is specified
    if !args.keep_database {
        if config.mapping.database_path.exists() {
            info!("Removing existing database to start fresh (use --keep-database to preserve mappings)");
            std::fs::remove_file(&config.mapping.database_path)?;
        }
    } else {
        info!("Keeping existing database mappings");
    }

    // Default to enabled for backward compatibility if no LLM config
    let ollama_config = config.llm.as_ref()
        .map(|llm| mcp_server_conceal_core::OllamaConfig {
            enabled: llm.enabled,
            endpoint: llm.endpoint.clone(),
            model: llm.model.clone(),
            timeout_seconds: llm.timeout_seconds,
        })
        .unwrap_or_else(|| mcp_server_conceal_core::OllamaConfig {
            enabled: true,
            endpoint: "http://localhost:11434".to_string(),
            model: "llama3.2:3b".to_string(),
            timeout_seconds: 30,
        });

    let proxy_config = mcp_server_conceal_core::IntegratedProxyConfig {
        target_command: args.target_command.clone(),
        target_args: args.parse_target_args(),
        target_env,
        target_cwd: args.target_cwd.clone(),
        config,
        ollama_config,
    };

    let mut proxy = mcp_server_conceal_core::IntegratedProxy::new(proxy_config)?;
    proxy.run().await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_args() -> Args {
        Args {
            target_command: "python".to_string(),
            target_args: None,
            target_env: vec![],
            target_cwd: None,
            log_level: "info".to_string(),
            config: None,
            keep_database: false,
        }
    }

    #[test]
    fn test_parse_target_args_empty() {
        let args = create_test_args();
        assert_eq!(args.parse_target_args(), Vec::<String>::new());
    }

    #[test]
    fn test_parse_target_args_simple() {
        let mut args = create_test_args();
        args.target_args = Some("server.py --port 3001".to_string());
        
        let expected = vec!["server.py".to_string(), "--port".to_string(), "3001".to_string()];
        assert_eq!(args.parse_target_args(), expected);
    }

    #[test]
    fn test_parse_target_args_quoted() {
        let mut args = create_test_args();
        args.target_args = Some(r#"server.py --config "path with spaces/config.json""#.to_string());
        
        let expected = vec![
            "server.py".to_string(), 
            "--config".to_string(), 
            "path with spaces/config.json".to_string()
        ];
        assert_eq!(args.parse_target_args(), expected);
    }

    #[test]
    fn test_parse_target_env_valid() {
        let mut args = create_test_args();
        args.target_env = vec![
            "API_KEY=secret123".to_string(),
            "DATABASE_URL=postgresql://localhost/test".to_string(),
        ];
        
        let env_map = args.parse_target_env().unwrap();
        assert_eq!(env_map.get("API_KEY"), Some(&"secret123".to_string()));
        assert_eq!(env_map.get("DATABASE_URL"), Some(&"postgresql://localhost/test".to_string()));
    }

    #[test]
    fn test_parse_target_env_invalid() {
        let mut args = create_test_args();
        args.target_env = vec!["INVALID_FORMAT".to_string()];
        
        assert!(args.parse_target_env().is_err());
    }
}