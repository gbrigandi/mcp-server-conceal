#!/usr/bin/env python3

import subprocess
import json
import sqlite3
import tempfile
import os
from pathlib import Path

PROJECT_ROOT = Path(__file__).parent.parent.parent
BINARY_PATH = PROJECT_ROOT / "target/release/mcp-server-conceal"
CONFIG_PATH = PROJECT_ROOT / "mcp-server-conceal.toml"
ECHO_SERVER_PATH = Path(__file__).parent / "test-servers/echo_server.py"

def clear_database(db_path="./mappings.db"):
    try:
        conn = sqlite3.connect(db_path)
        conn.execute("DELETE FROM entity_mappings WHERE 1=1")
        conn.execute("DELETE FROM llm_cache WHERE 1=1")
        conn.commit()
        conn.close()
    except Exception:
        pass

def run_mcp_conceal_test(payload, config_file=None, timeout=30):
    config = config_file or str(CONFIG_PATH)
    
    cmd = [
        str(BINARY_PATH),
        "--target-command", "python3",
        "--target-args", str(ECHO_SERVER_PATH),
        "--config", config,
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
        
        request_json = json.dumps(payload) + "\n"
        stdout, stderr = process.communicate(input=request_json, timeout=timeout)
        
        return {
            'stdout': stdout,
            'stderr': stderr,
            'returncode': process.returncode
        }
        
    except subprocess.TimeoutExpired:
        process.kill()
        return {
            'stdout': '',
            'stderr': 'Process timed out',
            'returncode': -1
        }
    except Exception as e:
        return {
            'stdout': '',
            'stderr': f'Error: {e}',
            'returncode': -1
        }

def create_test_payload(message, method="tools/call", name="echo"):
    return {
        "jsonrpc": "2.0",
        "id": 1,
        "method": method,
        "params": {
            "name": name,
            "arguments": {
                "message": message
            }
        }
    }

def extract_metrics_from_logs(stderr):
    import re
    
    metrics = {
        'mappings': 0,
        'cache_entries': 0,
        'entity_types': 0,
        'processing_confirmed': False
    }
    
    match = re.search(r"Total mappings created: (\d+)", stderr)
    if match:
        metrics['mappings'] = int(match.group(1))
    
    match = re.search(r"Cache entries: (\d+)", stderr)
    if match:
        metrics['cache_entries'] = int(match.group(1))
    
    metrics['processing_confirmed'] = "PII detected and anonymized" in stderr
    
    return metrics

def get_current_mappings(db_path="./mappings.db"):
    mappings = {}
    try:
        conn = sqlite3.connect(db_path)
        cursor = conn.execute("""
            SELECT entity_type, original_value_hash, fake_value 
            FROM entity_mappings 
            ORDER BY entity_type, created_at
        """)
        
        for entity_type, hash_val, fake_val in cursor.fetchall():
            if entity_type not in mappings:
                mappings[entity_type] = {}
            mappings[entity_type][hash_val] = fake_val
            
        conn.close()
    except Exception:
        pass
        
    return mappings