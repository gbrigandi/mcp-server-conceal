/*
   Manages prompt template loading with built-in fallback and user customization.
   Provides hybrid approach where templates can be embedded, auto-generated, or custom.
*/

use anyhow::Result;
use std::path::PathBuf;
use tracing::warn;
use crate::config::Config;

const BUILTIN_PROMPT: &str = include_str!("templates/builtin_prompt.md");

#[derive(Clone)]
pub struct PromptLoader {
    prompts_dir: PathBuf,
}

impl PromptLoader {
    pub fn new() -> Result<Self> {
        let project_dirs = Config::get_app_dirs()?;
        let data_dir = project_dirs.data_dir();
        let prompts_dir = data_dir.join("prompts");
        
        std::fs::create_dir_all(&prompts_dir)?;
        
        let default_prompt_path = prompts_dir.join("default.md");
        if !default_prompt_path.exists() {
            std::fs::write(&default_prompt_path, BUILTIN_PROMPT)?;
        }
        
        Ok(Self { prompts_dir })
    }
    
    pub fn load_prompt(&self, template_name: Option<&String>) -> Result<String> {
        match template_name {
            None => Ok(BUILTIN_PROMPT.to_string()),
            Some(name) => {
                let prompt_path = self.prompts_dir.join(format!("{}.md", name));
                match std::fs::read_to_string(&prompt_path) {
                    Ok(content) => Ok(content),
                    Err(_) => {
                        warn!("Prompt template '{}' not found, using built-in", name);
                        Ok(BUILTIN_PROMPT.to_string())
                    }
                }
            }
        }
    }
    
    pub fn format_prompt(&self, template: &str, text: &str) -> String {
        template.replace("{text}", &text.replace('"', r#"\""#))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Config;

    #[test]
    fn test_builtin_prompt_loading() {
        let loader = PromptLoader::new().unwrap();
        let prompt = loader.load_prompt(None).unwrap();
        
        assert!(prompt.contains("person_name"));
        assert!(prompt.contains("hostname"));
        assert!(prompt.contains("node_name"));
        assert!(prompt.contains("Built-in PII Detection Prompt"));
        println!("✓ Built-in prompt loaded: {} chars", prompt.len());
    }

    #[test]
    fn test_default_prompt_creation() {
        let loader = PromptLoader::new().unwrap();
        let prompt = loader.load_prompt(Some(&"default".to_string())).unwrap();
        
        assert!(prompt.contains("person_name"));
        assert!(prompt.contains("hostname"));
        println!("✓ Default prompt loaded: {} chars", prompt.len());
    }

    #[test]
    fn test_custom_prompt_loading() {
        let loader = PromptLoader::new().unwrap();
        let prompt = loader.load_prompt(Some(&"custom".to_string())).unwrap();
        
        if prompt.contains("Custom PII Detection Template") {
            println!("✓ Custom prompt loaded: {} chars", prompt.len());
            assert!(prompt.contains("simplified detection for testing"));
        } else {
            println!("✓ Custom prompt not found, fell back to built-in");
            assert!(prompt.contains("Built-in PII Detection Prompt"));
        }
    }

    #[test]
    fn test_nonexistent_prompt_fallback() {
        let loader = PromptLoader::new().unwrap();
        let prompt = loader.load_prompt(Some(&"nonexistent123".to_string())).unwrap();
        
        assert!(prompt.contains("Built-in PII Detection Prompt"));
        println!("✓ Nonexistent prompt fell back to built-in");
    }

    #[test]
    fn test_prompt_formatting() {
        let loader = PromptLoader::new().unwrap();
        let template = "TEXT: \"{text}\" - END";
        let formatted = loader.format_prompt(template, "test@example.com");
        
        assert_eq!(formatted, "TEXT: \"test@example.com\" - END");
        println!("✓ Prompt formatting works correctly");
    }

    #[test]
    fn test_data_directory_creation() {
        let dirs = Config::get_app_dirs().unwrap();
        let prompts_dir = dirs.data_dir().join("prompts");
        
        assert!(prompts_dir.exists());
        println!("✓ Prompts directory exists: {}", prompts_dir.display());
        
        let default_path = prompts_dir.join("default.md");
        if default_path.exists() {
            println!("✓ default.md exists");
        }
    }
}
