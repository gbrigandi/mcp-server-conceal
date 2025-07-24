# Built-in PII Detection Prompt

JSON_MODE_ONLY

TEXT: "{text}"

OUTPUT_REQUIREMENT: Return ONLY valid JSON. NO explanations. NO text. NO markdown. ONLY JSON.

FORMAT: {{"entities": [{{"type": "person_name", "value": "found_name", "start": 0, "end": 0, "confidence": 0.9}}, {{"type": "email", "value": "found_email", "start": 0, "end": 0, "confidence": 0.9}}]}}

EMPTY_RESULT: {{"entities": []}}

CRITICAL: Find ALL entities in the text. Scan for multiple entity types.

## Entity Types

### 1. person_name
Full names of people, first and last names together.
**EXAMPLES:** Sarah Johnson, John Smith, Maria Garcia, David Chen

### 2. email
Email addresses in any format.
**EXAMPLES:** sarah@company.com, john.smith@example.org, user+tag@domain.co.uk

### 3. phone
Phone numbers in various formats.
**EXAMPLES:** 555-123-4567, (555) 123-4567, 555.123.4567, +1-555-123-4567

### 4. ssn
Social Security Numbers.
**EXAMPLES:** 123-45-6789, 123 45 6789, 123456789

### 5. ip_address
IP addresses (IPv4 and IPv6).
**EXAMPLES:** 192.168.1.1, 10.0.0.1, 2001:db8::1

### 6. hostname
Computer hostnames, server names, machine names, system identifiers.

**HOSTNAME PATTERNS (classify as "hostname"):**
* Operating system names with dashes and numbers: ubuntu-linux-2404, centos-server-8, debian-box-1804
* Distribution names with version numbers: fedora-workstation-35, arch-linux-rolling, ubuntu-desktop-2204
* Server/system names with multiple dashes: web-server-prod-01, db-cluster-primary-03
* Machine identifiers with OS-like naming: windows-desktop-2022, macos-laptop-13
* Multi-component system names: backup-storage-nas-2023, mail-exchange-server-16
* Complex system identifiers: wazuh-manager-01, elastic-node-03, monitoring-host-12

### 7. node_name
Cluster nodes, compute nodes, network nodes in distributed systems.

**NODE NAME PATTERNS (classify as "node_name"):**
* "node" + numbers: node01, node1, node-1, node_01, node042, Node01, NODE01
* "worker" + numbers: worker01, worker-1, worker_05, Worker01
* "master" + numbers: master01, master-1, master_03, Master01
* "compute" + numbers: compute01, compute-node-5, Compute01
* "edge" + numbers: edge01, edge-device-12, Edge01
* Standalone "Node:" labels: look for "Node: nodeXX" patterns in text

## Critical Detection Rules

- Look for "Node:" followed by any identifier → the identifier is node_name
- Any word containing "node" + digits → node_name (case insensitive)
- Any word containing OS names (ubuntu, centos, debian, fedora, rhel, windows) → hostname
- Complex multi-dash names with OS/system terms → hostname
- Simple prefix + number patterns → likely node_name
- Pay special attention to structured data with labels like "Node: xxx"

SCAN THOROUGHLY: Check every word in the text for ALL entity types. Do not stop after finding one entity type.

## Specific Wazuh Agent Format Examples

Multi-line agent data like this:
```
Agent ID: 000 (Wazuh Manager)
Name: ubuntu-linux-2404
Status: 🟢 ACTIVE
IP: 105.65.172.228
OS: Ubuntu 24.04.2 LTS (aarch64)
Agent Version: Wazuh v4.12.0
Last Keep Alive: 9999-12-31T23:59:59+00:00
Registered: 2025-05-12T16:10:06+00:00
Node: node01
Config Status: ✅ SYNCED
```

**FROM THIS TEXT EXTRACT:**
- "Name: ubuntu-linux-2404" → extract "ubuntu-linux-2404" as hostname
- "Node: node01" → extract "node01" as node_name
- "IP: 105.65.172.228" → extract "105.65.172.228" as ip_address

## Critical Multi-line Parsing Rules

1. Look for "Name:" followed by any system identifier → hostname
2. Look for "Node:" followed by any identifier → node_name
3. Scan each line separately for patterns
4. Handle newlines and special characters (🟢, ✅)
5. Parse structured key-value pairs with colons

## Processing Strategy

1. Split text by newlines and scan each line
2. Look for "Name:" and extract the hostname value
3. Look for "Node:" and extract the node_name value
4. Find IP addresses on lines starting with "IP:"
5. Detect OS-related hostnames with version numbers

## Expected JSON Output Format

```json
{{"entities": [
  {{"type": "hostname", "value": "ubuntu-linux-2404", "start": 0, "end": 0, "confidence": 0.95}},
  {{"type": "node_name", "value": "node01", "start": 0, "end": 0, "confidence": 0.95}},
  {{"type": "ip_address", "value": "105.65.172.228", "start": 0, "end": 0, "confidence": 0.95}}
]}}
```

**IMPORTANT:** Always return complete JSON with all found entities. If no entities found, return: `{{"entities": []}}`

Return valid JSON only: