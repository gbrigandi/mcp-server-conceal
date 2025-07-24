#!/usr/bin/env python3
"""
Quick benchmark analysis with key performance metrics
"""

import subprocess
import json
import time
import sqlite3
import re

def quick_benchmark():
    print("⚡ MCP-SERVER-CONCEAL PERFORMANCE BENCHMARK")
    print("="*50)
    
    # Test different payload sizes
    test_cases = [
        ("Small", 67, '{"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": "echo", "arguments": {"message": "Contact test@example.com"}}}'),
        ("Medium", 456, '{"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": "echo", "arguments": {"message": "Customer: Sarah Johnson\\nEmail: sarah@company.com\\nPhone: (555) 123-4567\\nAgent: support@company.com\\nNotes: Contact Sarah Johnson at sarah@company.com or (555) 123-4567 for follow-up.\\nSecondary: john@email.org"}}}'),
        ("Large", 1200, '{"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": "echo", "arguments": {"message": "MEDICAL RECORD\\nPatient: Dr. Sarah Johnson\\nDOB: 1985-03-15\\nEmail: sarah@company.com\\nPhone: (555) 123-4567\\nEmergency: John Smith\\nContact: john@email.org\\nPhone: (555) 987-6543\\nPhysician: Dr. Brown\\nEmail: dr.brown@hospital.org\\nNotes: Patient Sarah Johnson contacted at sarah@company.com. Emergency contact John Smith at john@email.org or (555) 987-6543. Follow-up with Dr. Brown at dr.brown@hospital.org.\\nBilling: billing@hospital.org\\nInsurance: claims@insurance.org"}}}')
    ]
    
    results = []
    
    for name, size, payload_json in test_cases:
        print(f"\n📊 {name} Payload ({size} chars)")
        print("-" * 30)
        
        # Clear database for clean metrics
        clear_db()
        
        # Single timing test
        start_time = time.time()
        metrics = run_single_test(payload_json)
        end_time = time.time()
        
        processing_time = end_time - start_time
        
        if metrics:
            throughput = size / processing_time if processing_time > 0 else 0
            
            results.append({
                'name': name,
                'size': size,
                'time': processing_time,
                'throughput': throughput,
                'mappings': metrics.get('mappings', 0),
                'ollama_calls': metrics.get('ollama_calls', 0)
            })
            
            print(f"Processing Time: {processing_time:.3f}s")
            print(f"Throughput: {throughput:.1f} chars/sec")
            print(f"Mappings Created: {metrics.get('mappings', 0)}")
            print(f"LLM Calls: {metrics.get('ollama_calls', 0)}")
        else:
            print("❌ Test failed")
    
    # Performance summary
    print("\n" + "="*50)
    print("📈 PERFORMANCE SUMMARY")
    print("="*50)
    
    if len(results) >= 2:
        for i, result in enumerate(results):
            print(f"\n{result['name']}:")
            print(f"  Size: {result['size']:,} chars")
            print(f"  Time: {result['time']:.3f}s")
            print(f"  Rate: {result['throughput']:.0f} chars/sec")
            print(f"  Entities: {result['mappings']}")
            
            if i > 0:
                prev = results[i-1]
                size_ratio = result['size'] / prev['size']
                time_ratio = result['time'] / prev['time']
                print(f"  Scaling: {size_ratio:.1f}x size → {time_ratio:.1f}x time")
    
    # Cache performance test
    print(f"\n💾 CACHE PERFORMANCE")
    print("-" * 30)
    
    test_payload = test_cases[0][2]  # Small payload
    
    # First request (miss)
    clear_db()
    start = time.time()
    run_single_test(test_payload)
    miss_time = time.time() - start
    
    # Second request (hit) 
    start = time.time()
    run_single_test(test_payload)
    hit_time = time.time() - start
    
    print(f"Cache MISS: {miss_time:.3f}s")
    print(f"Cache HIT:  {hit_time:.3f}s")
    
    if miss_time > hit_time:
        speedup = miss_time / hit_time
        print(f"Cache Speedup: {speedup:.2f}x")
    
    # Database stats
    try:
        conn = sqlite3.connect("./mappings.db")
        cache_count = conn.execute("SELECT COUNT(*) FROM llm_cache").fetchone()[0]
        mapping_count = conn.execute("SELECT COUNT(*) FROM entity_mappings").fetchone()[0]
        conn.close()
        
        print(f"DB Cache Entries: {cache_count}")
        print(f"DB Mappings: {mapping_count}")
    except:
        pass

def clear_db():
    try:
        conn = sqlite3.connect("./mappings.db")
        conn.execute("DELETE FROM entity_mappings")
        conn.execute("DELETE FROM llm_cache")
        conn.commit()
        conn.close()
    except:
        pass

def run_single_test(payload_json):
    cmd = [
        "./target/release/mcp-server-conceal",
        "--target-command", "python3",
        "--target-args", "tests/python/test-servers/echo_server.py",
        "--config", "mcp-server-conceal.toml",
        "--log-level", "info"
    ]
    
    try:
        process = subprocess.Popen(
            cmd,
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True
        )
        
        stdout, stderr = process.communicate(input=payload_json + "\n", timeout=15)
        
        # Extract metrics
        metrics = {}
        
        # Mappings created
        match = re.search(r"Total mappings created: (\d+)", stderr)
        if match:
            metrics['mappings'] = int(match.group(1))
        
        # Ollama calls
        metrics['ollama_calls'] = stderr.count("Ollama extracted")
        
        return metrics
        
    except subprocess.TimeoutExpired:
        process.kill()
        return None
    except Exception as e:
        print(f"Error: {e}")
        return None

if __name__ == "__main__":
    quick_benchmark()
