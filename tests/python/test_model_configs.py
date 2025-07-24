#!/usr/bin/env python3
"""
Test different Ollama model configurations
"""

import subprocess
import json
import tempfile
import os

def test_model_configurations():
    print("🤖 TESTING CONFIGURABLE OLLAMA MODELS")
    print("="*50)
    
    # Different model configurations to test
    model_configs = [
        {
            "name": "Llama 3.2 3B (Fast)",
            "model": "llama3.2:3b",
            "description": "Lightweight, fast processing"
        },
        {
            "name": "Llama 3.2 1B (Ultra-fast)", 
            "model": "llama3.2:1b",
            "description": "Smallest model, fastest processing"
        },
        {
            "name": "Llama 3.1 8B (Accurate)",
            "model": "llama3.1:8b", 
            "description": "Larger model, better accuracy"
        },
        {
            "name": "CodeLlama 7B (Code-focused)",
            "model": "codellama:7b",
            "description": "Optimized for code and structured text"
        }
    ]
    
    base_config = """# mcp-server-conceal Configuration File

[detection]
mode = "regex"
enabled = true
confidence_threshold = 0.8

[detection.patterns]
email = "\\\\b[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\\\\.[A-Z|a-z]{{2,}}\\\\b"
phone = "\\\\b(?:\\\\+?1[-\\\\.\\\\s]?)?(?:\\\\(?[0-9]{{3}}\\\\)?[-\\\\.\\\\s]?)?[0-9]{{3}}[-\\\\.\\\\s]?[0-9]{{4}}\\\\b"

[faker]
locale = "en_US"
seed = 12345
consistency = true

[mapping]
database_path = "./test_mappings.db"
encryption = false
retention_days = 90

[llm]
enabled = true
model = "{model}"
endpoint = "http://localhost:11434"
timeout_seconds = 30
"""
    
    test_request = {
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call", 
        "params": {
            "name": "echo",
            "arguments": {
                "message": "Contact support@company.com or call (555) 123-4567 for assistance."
            }
        }
    }
    
    for config in model_configs:
        print(f"\n🔧 Testing {config['name']}")
        print(f"   Model: {config['model']}")
        print(f"   {config['description']}")
        print("-" * 40)
        
        # Create temporary config file with this model
        config_content = base_config.format(model=config['model'])
        
        with tempfile.NamedTemporaryFile(mode='w', suffix='.toml', delete=False) as f:
            f.write(config_content)
            config_file = f.name
        
        try:
            # Test model availability first
            availability = check_model_availability(config['model'])
            
            if availability == "available":
                print("   ✅ Model is available")
                # Run actual test
                result = run_test_with_config(config_file, test_request)
                if result:
                    print(f"   ✅ Processing successful: {result}")
                else:
                    print("   ⚠️ Processing failed or timed out")
            elif availability == "not_pulled":
                print(f"   📥 Model not pulled. Run: ollama pull {config['model']}")
            else:
                print("   ❌ Ollama not available")
                
        finally:
            # Clean up temp file
            try:
                os.unlink(config_file)
            except:
                pass
        
        print()
    
    # Show current configuration
    print("="*50)
    print("📋 CURRENT CONFIGURATION:")
    try:
        with open("mcp-server-conceal.toml", 'r') as f:
            lines = f.readlines()
            in_llm_section = False
            for line in lines:
                if line.strip().startswith("[llm]"):
                    in_llm_section = True
                elif line.strip().startswith("[") and in_llm_section:
                    break
                elif in_llm_section:
                    print(f"   {line.rstrip()}")
    except FileNotFoundError:
        print("   No configuration file found")

def check_model_availability(model):
    """Check if model is available in Ollama"""
    try:
        result = subprocess.run(['ollama', 'list'], 
                              capture_output=True, text=True, timeout=5)
        if result.returncode == 0:
            if model in result.stdout:
                return "available"
            else:
                return "not_pulled"
        else:
            return "ollama_error"
    except (subprocess.TimeoutExpired, FileNotFoundError):
        return "ollama_not_available"

def run_test_with_config(config_file, request):
    """Run a quick test with the given configuration"""
    cmd = [
        "./target/release/mcp-server-conceal",
        "--target-command", "python3",
        "--target-args", "tests/python/test-servers/echo_server.py",
        "--config", config_file,
        "--log-level", "warn"
    ]
    
    try:
        process = subprocess.Popen(
            cmd,
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True
        )
        
        request_json = json.dumps(request) + "\n"
        stdout, stderr = process.communicate(input=request_json, timeout=15)
        
        # Check if processing occurred
        if "PII detected and anonymized" in stderr or "Total mappings created:" in stderr:
            return "PII processing completed"
        elif "Ollama health check passed" in stderr:
            return "Ollama connection successful"
        else:
            return None
            
    except subprocess.TimeoutExpired:
        process.kill()
        return None
    except Exception as e:
        return f"Error: {e}"

if __name__ == "__main__":
    test_model_configurations()