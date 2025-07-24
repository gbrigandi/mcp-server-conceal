#!/usr/bin/env python3

import json
import sys
import argparse
import time
import random
from typing import Dict, Any, Optional


class ErrorMCPServer:
    def __init__(self, 
                 crash_after: Optional[int] = None,
                 timeout_requests: bool = False,
                 malformed_responses: bool = False,
                 random_errors: bool = False,
                 error_rate: float = 0.3):
        self.crash_after = crash_after
        self.timeout_requests = timeout_requests
        self.malformed_responses = malformed_responses
        self.random_errors = random_errors
        self.error_rate = error_rate
        self.request_count = 0
        
    def log(self, message: str):
        print(f"[ErrorServer] {message}", file=sys.stderr, flush=True)
        
    def handle_initialize(self, request: Dict[str, Any]) -> Dict[str, Any]:
        return {
            "jsonrpc": "2.0",
            "id": request.get("id"),
            "result": {
                "protocolVersion": "2024-11-05",
                "capabilities": {
                    "tools": {},
                    "resources": {},
                    "prompts": {},
                },
                "serverInfo": {
                    "name": "error-server",
                    "version": "1.0.0"
                }
            }
        }
    
    def should_introduce_error(self) -> bool:
        if self.random_errors:
            return random.random() < self.error_rate
        return False
    
    def generate_error_response(self, request: Dict[str, Any]) -> Dict[str, Any]:
        error_types = [
            {
                "code": -32603,
                "message": "Internal error"
            },
            {
                "code": -32601,
                "message": "Method not found"
            },
            {
                "code": -32602,
                "message": "Invalid params"
            },
            {
                "code": -32000,
                "message": "Server error: Simulated failure"
            }
        ]
        
        error = random.choice(error_types)
        return {
            "jsonrpc": "2.0",
            "id": request.get("id"),
            "error": error
        }
    
    def generate_malformed_response(self) -> str:
        malformed_types = [
            '{"jsonrpc": "2.0", "id": 1, "result": {',
            'not-json-at-all',
            '{"jsonrpc": "2.0"}',
            '',
            '{"jsonrpc": "2.0", "id": 1, "result": {"key": "unclosed string}',
        ]
        
        return random.choice(malformed_types)
    
    def handle_request(self, request: Dict[str, Any]) -> Optional[str]:
        self.request_count += 1
        method = request.get("method", "")
        
        self.log(f"Request #{self.request_count}: {method}")
        
        if self.crash_after and self.request_count >= self.crash_after:
            self.log(f"Crashing after {self.request_count} requests")
            sys.exit(1)
        
        if self.timeout_requests:
            self.log("Simulating timeout - not responding")
            time.sleep(30)
            return None
        
        if self.malformed_responses:
            self.log("Returning malformed response")
            return self.generate_malformed_response()
        
        if self.should_introduce_error():
            self.log("Introducing random error")
            response = self.generate_error_response(request)
            return json.dumps(response)
        
        if method == "initialize":
            response = self.handle_initialize(request)
            return json.dumps(response)
        else:
            response = {
                "jsonrpc": "2.0",
                "id": request.get("id"),
                "result": {
                    "message": f"Processed {method}",
                    "request_count": self.request_count
                }
            }
            return json.dumps(response)
    
    def run(self):
        self.log("Starting Error MCP Server")
        self.log(f"Config: crash_after={self.crash_after}, timeout={self.timeout_requests}, "
               f"malformed={self.malformed_responses}, random_errors={self.random_errors}")
        
        try:
            for line in sys.stdin:
                line = line.strip()
                if not line:
                    continue
                
                try:
                    request = json.loads(line)
                    response_str = self.handle_request(request)
                    
                    if response_str is not None:
                        print(response_str, flush=True)
                    
                except json.JSONDecodeError as e:
                    self.log(f"JSON decode error: {e}")
                    error_response = {
                        "jsonrpc": "2.0",
                        "id": None,
                        "error": {
                            "code": -32700,
                            "message": "Parse error"
                        }
                    }
                    print(json.dumps(error_response), flush=True)
                    
        except KeyboardInterrupt:
            self.log("Shutting down")
        except Exception as e:
            self.log(f"Unexpected error: {e}")
            sys.exit(1)


def main():
    parser = argparse.ArgumentParser(description="Mock Error MCP Server")
    parser.add_argument("--crash-after", type=int,
                       help="Crash after N requests")
    parser.add_argument("--timeout", action="store_true",
                       help="Never respond to requests (simulate timeout)")
    parser.add_argument("--malformed", action="store_true",
                       help="Return malformed responses")
    parser.add_argument("--random-errors", action="store_true",
                       help="Randomly return error responses")
    parser.add_argument("--error-rate", type=float, default=0.3,
                       help="Error rate for random errors (0.0-1.0, default: 0.3)")
    parser.add_argument("--verbose", "-v", action="store_true",
                       help="Enable verbose logging")
    
    args = parser.parse_args()
    
    server = ErrorMCPServer(
        crash_after=args.crash_after,
        timeout_requests=args.timeout,
        malformed_responses=args.malformed,
        random_errors=args.random_errors,
        error_rate=args.error_rate
    )
    server.run()


if __name__ == "__main__":
    main()