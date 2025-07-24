#!/usr/bin/env python3
"""
Benchmarking analysis of mcp-server-conceal performance
"""

import subprocess
import json
import time
import sqlite3
import statistics
import re
from datetime import datetime

def benchmark_analysis():
    print("🔬 MCP-SERVER-CONCEAL PERFORMANCE BENCHMARKING ANALYSIS")
    print("="*70)
    
    # Test payloads of different sizes
    test_cases = [
        ("small", create_small_payload()),
        ("medium", create_medium_payload()),
        ("large", create_large_payload()),
    ]
    
    results = []
    
    for size_name, payload in test_cases:
        print(f"\n📊 TESTING {size_name.upper()} PAYLOAD")
        print("-" * 50)
        
        payload_size = len(json.dumps(payload))
        print(f"Payload size: {payload_size:,} characters")
        
        # Clear database for clean metrics
        clear_database()
        
        # Run multiple iterations for statistical analysis
        times = []
        mapping_counts = []
        
        for i in range(5):  # 5 iterations per payload size
            start_time = time.time()
            
            result = run_timed_request(payload, f"{size_name}_{i+1}")
            
            end_time = time.time()
            processing_time = end_time - start_time
            times.append(processing_time)
            
            if result:
                mapping_counts.append(result['mappings'])
            
        # Calculate statistics
        if times:
            avg_time = statistics.mean(times)
            min_time = min(times)
            max_time = max(times)
            std_dev = statistics.stdev(times) if len(times) > 1 else 0
            
            avg_mappings = statistics.mean(mapping_counts) if mapping_counts else 0
            
            results.append({
                'size': size_name,
                'payload_size': payload_size,
                'avg_time': avg_time,
                'min_time': min_time,
                'max_time': max_time,
                'std_dev': std_dev,
                'avg_mappings': avg_mappings,
                'throughput': payload_size / avg_time if avg_time > 0 else 0
            })
            
            print(f"Average processing time: {avg_time:.3f}s")
            print(f"Min/Max time: {min_time:.3f}s / {max_time:.3f}s")
            print(f"Standard deviation: {std_dev:.3f}s")
            print(f"Average mappings created: {avg_mappings:.1f}")
            print(f"Throughput: {payload_size / avg_time:.1f} chars/sec")
    
    # Performance analysis
    print("\n" + "="*70)
    print("📈 PERFORMANCE ANALYSIS SUMMARY")
    print("="*70)
    
    analyze_performance_trends(results)
    analyze_caching_performance()
    analyze_database_performance()
    analyze_ollama_performance()

def create_small_payload():
    return {
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": {
            "name": "echo",
            "arguments": {
                "message": "Contact john@test.com or call (555) 123-4567"
            }
        }
    }

def create_medium_payload():
    return {
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": {
            "name": "process_data",
            "arguments": {
                "document": """
                Customer Support Ticket #12345
                
                Customer: Sarah Johnson
                Email: sarah.johnson@company.com
                Phone: (555) 123-4567
                
                Issue: Need help with account access
                Priority: High
                
                Contact History:
                - Called (555) 123-4567 on Monday
                - Emailed sarah.johnson@company.com on Tuesday
                - Follow-up with Sarah Johnson scheduled
                
                Agent Notes:
                - Customer very patient
                - Resolved via phone call to (555) 123-4567
                - Send confirmation to sarah.johnson@company.com
                """,
                "metadata": {
                    "customer_email": "sarah.johnson@company.com",
                    "customer_phone": "(555) 123-4567",
                    "agent": "support@company.com"
                }
            }
        }
    }

def create_large_payload():
    return {
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": {
            "name": "process_medical_record",
            "arguments": {
                "medical_record": """
                CONFIDENTIAL MEDICAL RECORD - PATIENT #98765
                ================================================
                
                PRIMARY PATIENT INFORMATION:
                Name: Dr. Sarah Elizabeth Johnson
                DOB: 1985-03-15
                Email: sarah.johnson@company.com
                Primary Phone: (555) 123-4567
                Secondary Phone: (555) 999-0000
                Address: 123 Main Street, Anytown, ST 12345
                SSN: 123-45-6789
                
                EMERGENCY CONTACT:
                Name: John Michael Smith
                Relationship: Spouse
                Email: john.smith@email.org
                Phone: (555) 987-6543
                
                INSURANCE INFORMATION:
                Provider: HealthCorp Insurance
                Policy: POL-123456789
                Group: GRP-987654321
                Contact: claims@healthcorp.org
                Phone: (800) 555-0123
                
                MEDICAL HISTORY:
                Patient Dr. Sarah Johnson has been under our care since 2020.
                Primary care physician: Dr. Michael Brown (m.brown@hospital.org)
                Specialist referrals: Dr. Jennifer Davis (j.davis@clinic.net)
                
                TREATMENT NOTES:
                - Patient contacted at sarah.johnson@company.com for appointment
                - Emergency contact John Smith reached at (555) 987-6543
                - Insurance verification completed with claims@healthcorp.org
                - Follow-up scheduled with Dr. Brown at m.brown@hospital.org
                - Patient Sarah Johnson's condition improving
                - Alternative contact method: (555) 999-0000
                
                BILLING INFORMATION:
                Billing contact: billing@hospital.org
                Patient portal: patient.portal@healthsystem.com
                Account ID: ACC-789012345
                
                ADDITIONAL CONTACTS:
                Pharmacy: rx@pharmacy.net, (555) 246-8101
                Lab results: lab@diagnostics.org
                Radiology: imaging@radcenter.com, (555) 135-7902
                """,
                "contacts": [
                    {"name": "Dr. Sarah Johnson", "email": "sarah.johnson@company.com", "role": "patient"},
                    {"name": "John Smith", "email": "john.smith@email.org", "role": "emergency"},
                    {"name": "Dr. Michael Brown", "email": "m.brown@hospital.org", "role": "physician"},
                    {"name": "Dr. Jennifer Davis", "email": "j.davis@clinic.net", "role": "specialist"}
                ],
                "medications": [
                    {"name": "Medication A", "prescriber": "Dr. Brown", "contact": "m.brown@hospital.org"},
                    {"name": "Medication B", "prescriber": "Dr. Davis", "contact": "j.davis@clinic.net"}
                ]
            }
        }
    }

def clear_database():
    try:
        conn = sqlite3.connect("./mappings.db")
        conn.execute("DELETE FROM entity_mappings WHERE 1=1")
        conn.execute("DELETE FROM llm_cache WHERE 1=1")
        conn.commit()
        conn.close()
    except:
        pass

def run_timed_request(payload, test_id):
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
        
        request_json = json.dumps(payload) + "\n"
        stdout, stderr = process.communicate(input=request_json, timeout=45)
        
        # Extract metrics from logs
        metrics = extract_metrics_from_logs(stderr)
        return metrics
        
    except subprocess.TimeoutExpired:
        process.kill()
        return None
    except Exception as e:
        print(f"Error in {test_id}: {e}")
        return None

def extract_metrics_from_logs(stderr):
    """Extract performance metrics from stderr logs"""
    metrics = {
        'mappings': 0,
        'cache_entries': 0,
        'entity_types': 0,
        'ollama_calls': 0,
        'processing_confirmed': False
    }
    
    # Extract total mappings
    match = re.search(r"Total mappings created: (\d+)", stderr)
    if match:
        metrics['mappings'] = int(match.group(1))
    
    # Extract cache entries
    match = re.search(r"Cache entries: (\d+)", stderr)
    if match:
        metrics['cache_entries'] = int(match.group(1))
    
    # Count entity types
    entity_type_match = re.search(r'Entity types processed: \{([^}]+)\}', stderr)
    if entity_type_match:
        entity_types = entity_type_match.group(1)
        # Count comma-separated items
        metrics['entity_types'] = len([x.strip() for x in entity_types.split(',') if x.strip()])
    
    # Count Ollama calls
    metrics['ollama_calls'] = stderr.count("Ollama extracted")
    
    # Check if processing was confirmed
    metrics['processing_confirmed'] = "PII detected and anonymized" in stderr
    
    return metrics

def analyze_performance_trends(results):
    print("\n🚀 PERFORMANCE TRENDS:")
    print("-" * 30)
    
    for result in results:
        print(f"\n{result['size'].upper()} PAYLOAD:")
        print(f"  Size: {result['payload_size']:,} chars")
        print(f"  Avg Time: {result['avg_time']:.3f}s")
        print(f"  Throughput: {result['throughput']:.1f} chars/sec")
        print(f"  Avg Mappings: {result['avg_mappings']:.1f}")
        print(f"  Time per mapping: {result['avg_time']/max(result['avg_mappings'], 1):.4f}s")
    
    # Scaling analysis
    if len(results) >= 2:
        small = results[0]
        large = results[-1]
        
        size_ratio = large['payload_size'] / small['payload_size']
        time_ratio = large['avg_time'] / small['avg_time']
        
        print(f"\n📊 SCALING ANALYSIS:")
        print(f"  Size increase: {size_ratio:.1f}x")
        print(f"  Time increase: {time_ratio:.1f}x")
        
        if time_ratio < size_ratio:
            print(f"  ✅ Sub-linear scaling: Good performance")
        else:
            print(f"  ⚠️ Super-linear scaling: Performance degradation")

def analyze_caching_performance():
    print(f"\n💾 CACHING PERFORMANCE:")
    print("-" * 30)
    
    # Test cache hit performance
    test_payload = create_small_payload()
    
    # First request (cache miss)
    clear_database()
    start = time.time()
    result1 = run_timed_request(test_payload, "cache_miss")
    miss_time = time.time() - start
    
    # Second identical request (cache hit)
    start = time.time()
    result2 = run_timed_request(test_payload, "cache_hit")
    hit_time = time.time() - start
    
    if result1 and result2:
        print(f"  Cache MISS time: {miss_time:.3f}s")
        print(f"  Cache HIT time:  {hit_time:.3f}s")
        
        if hit_time < miss_time:
            speedup = miss_time / hit_time
            print(f"  ✅ Cache speedup: {speedup:.2f}x faster")
        else:
            print(f"  ⚠️ No cache benefit detected")
    
    # Check database cache entries
    try:
        conn = sqlite3.connect("./mappings.db")
        cursor = conn.execute("SELECT COUNT(*) FROM llm_cache")
        cache_count = cursor.fetchone()[0]
        
        cursor = conn.execute("SELECT COUNT(*) FROM entity_mappings") 
        mapping_count = cursor.fetchone()[0]
        
        conn.close()
        
        print(f"  LLM cache entries: {cache_count}")
        print(f"  Entity mappings: {mapping_count}")
        
    except Exception as e:
        print(f"  Database query error: {e}")

def analyze_database_performance():
    print(f"\n🗄️ DATABASE PERFORMANCE:")
    print("-" * 30)
    
    # Test database operations under load
    payload_sizes = [100, 500, 1000]
    
    for size in payload_sizes:
        # Create payload with specific number of entities
        payload = create_payload_with_entities(size // 100)  # Roughly 100 chars per entity
        
        start = time.time()
        run_timed_request(payload, f"db_load_{size}")
        db_time = time.time() - start
        
        print(f"  {size} chars payload: {db_time:.3f}s")
    
    # Check database size
    try:
        import os
        if os.path.exists("./mappings.db"):
            size = os.path.getsize("./mappings.db")
            print(f"  Database size: {size:,} bytes")
    except:
        pass

def create_payload_with_entities(entity_count):
    """Create payload with specific number of entities"""
    entities = []
    for i in range(entity_count):
        entities.append(f"Contact user{i}@test{i}.com or call (555) {100+i:03d}-{1000+i:04d}")
    
    return {
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": {
            "name": "echo",
            "arguments": {
                "message": " ".join(entities)
            }
        }
    }

def analyze_ollama_performance():
    print(f"\n🤖 OLLAMA LLM PERFORMANCE:")
    print("-" * 30)
    
    # Test Ollama response times with different text lengths
    test_texts = [
        "Short text with john@test.com",
        "Medium length text with multiple entities: john@test.com, (555) 123-4567, and Sarah Johnson work together.",
        "Long text with many entities: " + " ".join([
            f"Employee{i} at employee{i}@company.com with phone (555) {100+i:03d}-{1000+i:04d}"
            for i in range(10)
        ])
    ]
    
    for i, text in enumerate(test_texts):
        payload = {
            "jsonrpc": "2.0", "id": 1, "method": "tools/call",
            "params": {"name": "echo", "arguments": {"message": text}}
        }
        
        clear_database()  # Force LLM calls
        
        start = time.time()
        result = run_timed_request(payload, f"ollama_test_{i}")
        ollama_time = time.time() - start
        
        text_length = len(text)
        print(f"  {text_length:3d} chars: {ollama_time:.3f}s")
        
        if result and result['ollama_calls'] > 0:
            print(f"    Ollama calls: {result['ollama_calls']}")
            print(f"    Time per call: {ollama_time/result['ollama_calls']:.3f}s")

if __name__ == "__main__":
    benchmark_analysis()
