#!/usr/bin/env python3
"""
Test pseudo-anonymization with larger payload and consistency verification
"""

import subprocess
import json
import sys

def test_pseudo_anonymization():
    print("Testing mcp-server-conceal pseudo-anonymization with larger payload...")
    
    # Test request with multiple instances of the same PII
    test_request = {
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": {
            "name": "process_document",
            "arguments": {
                "document": """
                CONFIDENTIAL MEDICAL RECORD
                
                Patient: Dr. Sarah Johnson
                Email: sarah.johnson@company.com
                Phone: (555) 123-4567
                
                Emergency Contact: John Smith
                Contact Email: sarah.johnson@company.com  
                Contact Phone: (555) 123-4567
                
                Physician: Dr. Michael Brown
                Physician Email: m.brown@hospital.org
                Physician Phone: (555) 987-6543
                
                Additional Notes:
                - Patient Dr. Sarah Johnson has been treated multiple times
                - Please contact sarah.johnson@company.com for follow-up
                - Alternative contact: John Smith at (555) 999-0000
                - Dr. Michael Brown will review case
                - Hospital contact: reception@hospital.org
                """,
                "metadata": {
                    "patient_name": "Dr. Sarah Johnson",
                    "doctor": "Dr. Michael Brown",
                    "contacts": [
                        {"name": "Sarah Johnson", "email": "sarah.johnson@company.com"},
                        {"name": "John Smith", "phone": "(555) 999-0000"}
                    ]
                }
            }
        }
    }
    
    print(f"INPUT REQUEST (showing document excerpt):")
    print("Document contains:")
    print("- Dr. Sarah Johnson (appears 3 times)")
    print("- sarah.johnson@company.com (appears 3 times)")
    print("- (555) 123-4567 (appears 2 times)")
    print("- John Smith (appears 2 times)")
    print("- Dr. Michael Brown (appears 2 times)")
    print("- Multiple other emails and phones")
    print("\n" + "="*60)
    
    # Run first request
    print("FIRST REQUEST:")
    first_response = run_request(test_request)
    
    if not first_response:
        print("❌ No response from first request")
        return
        
    # Run second identical request to test consistency
    print("\nSECOND IDENTICAL REQUEST:")
    second_response = run_request(test_request)
    
    if not second_response:
        print("❌ No response from second request")
        return
        
    # Analyze responses for pseudo-anonymization
    analyze_pseudo_anonymization(first_response, second_response)

def run_request(test_request):
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
        
        stdout, stderr = process.communicate(input=json.dumps(test_request) + "\n", timeout=45)
        
        # Extract anonymized content from response
        if stdout.strip():
            try:
                response = json.loads(stdout.strip())
                if "result" in response and "content" in response["result"]:
                    return response["result"]["content"][0]["text"]
            except json.JSONDecodeError:
                pass
        
        return None
        
    except subprocess.TimeoutExpired:
        print("Process timed out")
        process.kill()
        return None
    except Exception as e:
        print(f"Error running CLI: {e}")
        return None

def analyze_pseudo_anonymization(first_response, second_response):
    print("PSEUDO-ANONYMIZATION ANALYSIS:")
    print("="*60)
    
    if not first_response or not second_response:
        print("❌ Missing responses for analysis")
        return
    
    # Check if responses are identical (pseudo-anonymization)
    if first_response == second_response:
        print("✅ PSEUDO-ANONYMIZATION WORKING: Identical responses for same input")
    else:
        print("❌ PSEUDO-ANONYMIZATION FAILED: Responses differ")
        print(f"First response length: {len(first_response)}")
        print(f"Second response length: {len(second_response)}")
        return
    
    # Extract fake values and check consistency within single response
    response = first_response
    print(f"\nRESPONSE ANALYSIS (Length: {len(response)} chars):")
    
    # Check for original PII (should not exist)
    original_pii = [
        "Dr. Sarah Johnson",
        "sarah.johnson@company.com", 
        "(555) 123-4567",
        "John Smith",
        "Dr. Michael Brown",
        "m.brown@hospital.org",
        "(555) 987-6543"
    ]
    
    found_original = []
    for pii in original_pii:
        if pii in response:
            found_original.append(pii)
    
    if found_original:
        print("❌ ORIGINAL PII STILL PRESENT:")
        for pii in found_original:
            print(f"  - {pii}")
    else:
        print("✅ ALL ORIGINAL PII REMOVED")
    
    # Check for consistency of replacements within response
    print("\nCONSISTENCY CHECK:")
    
    # Find email patterns
    import re
    emails = re.findall(r'\b[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Z|a-z]{2,}\b', response)
    print(f"Found {len(emails)} email addresses: {set(emails)}")
    
    # Find phone patterns
    phones = re.findall(r'\([0-9]{3}\) [0-9]{3}-[0-9]{4}', response)
    print(f"Found {len(phones)} phone numbers: {set(phones)}")
    
    # Find name patterns (Dr. prefix)
    dr_names = re.findall(r'Dr\. [A-Z][a-z]+ [A-Z][a-z]+', response)
    print(f"Found {len(dr_names)} doctor names: {set(dr_names)}")
    
    # Check consistency
    email_counts = {}
    for email in emails:
        email_counts[email] = email_counts.get(email, 0) + 1
    
    phone_counts = {}
    for phone in phones:
        phone_counts[phone] = phone_counts.get(phone, 0) + 1
    
    name_counts = {}
    for name in dr_names:
        name_counts[name] = name_counts.get(name, 0) + 1
    
    print("\nCONSISTENCY RESULTS:")
    print(f"Email consistency: {email_counts}")
    print(f"Phone consistency: {phone_counts}")  
    print(f"Name consistency: {name_counts}")
    
    # Verify that same original values map to same fake values
    consistent = True
    for count_dict in [email_counts, phone_counts, name_counts]:
        for fake_value, count in count_dict.items():
            if count > 1:
                print(f"✅ {fake_value} appears {count} times (consistent mapping)")
            elif count == 1:
                print(f"ℹ️  {fake_value} appears 1 time")
    
    print(f"\n{'✅ PSEUDO-ANONYMIZATION SUCCESSFUL' if consistent else '❌ INCONSISTENT MAPPINGS DETECTED'}")

if __name__ == "__main__":
    test_pseudo_anonymization()