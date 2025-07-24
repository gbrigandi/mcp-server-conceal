#!/usr/bin/env python3

import json
import sys
import argparse
from typing import Dict, Any


class EchoMCPServer:
    def __init__(self, delay: float = 0.0):
        self.delay = delay
        self.request_count = 0
        
    def log(self, message: str):
        print(f"[EchoServer] {message}", file=sys.stderr, flush=True)
        
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
                    "roots": {
                        "listChanged": True
                    }
                },
                "serverInfo": {
                    "name": "echo-server",
                    "version": "1.0.0"
                }
            }
        }
    
    def handle_tools_list(self, request: Dict[str, Any]) -> Dict[str, Any]:
        return {
            "jsonrpc": "2.0",
            "id": request.get("id"),
            "result": {
                "tools": [
                    {
                        "name": "echo",
                        "description": "Echo back the input",
                        "inputSchema": {
                            "type": "object",
                            "properties": {
                                "message": {
                                    "type": "string",
                                    "description": "Message to echo"
                                }
                            },
                            "required": ["message"]
                        }
                    }
                ]
            }
        }
    
    def handle_tools_call(self, request: Dict[str, Any]) -> Dict[str, Any]:
        params = request.get("params", {})
        name = params.get("name", "")
        arguments = params.get("arguments", {})
        
        if name == "echo":
            message = arguments.get("message", "No message provided")
            return {
                "jsonrpc": "2.0",
                "id": request.get("id"),
                "result": {
                    "content": [
                        {
                            "type": "text",
                            "text": f"Echo: {message}"
                        }
                    ]
                }
            }
        else:
            return {
                "jsonrpc": "2.0",
                "id": request.get("id"),
                "error": {
                    "code": -32601,
                    "message": f"Unknown tool: {name}"
                }
            }
    
    def handle_request(self, request: Dict[str, Any]) -> Dict[str, Any]:
        self.request_count += 1
        method = request.get("method", "")
        
        self.log(f"Request #{self.request_count}: {method}")
        
        if method == "initialize":
            return self.handle_initialize(request)
        elif method == "tools/list":
            return self.handle_tools_list(request)
        elif method == "tools/call":
            return self.handle_tools_call(request)
        else:
            return {
                "jsonrpc": "2.0",
                "id": request.get("id"),
                "result": {
                    "echo": request,
                    "server": "echo-mcp-server",
                    "request_count": self.request_count
                }
            }
    
    def run(self):
        self.log("Starting Echo MCP Server")
        
        try:
            for line in sys.stdin:
                line = line.strip()
                if not line:
                    continue
                
                try:
                    request = json.loads(line)
                    response = self.handle_request(request)
                    
                    if self.delay > 0:
                        import time
                        time.sleep(self.delay)
                    
                    print(json.dumps(response), flush=True)
                    
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
    parser = argparse.ArgumentParser(description="Mock Echo MCP Server")
    parser.add_argument("--delay", type=float, default=0.0,
                       help="Delay in seconds before responding (default: 0.0)")
    parser.add_argument("--verbose", "-v", action="store_true",
                       help="Enable verbose logging")
    
    args = parser.parse_args()
    
    server = EchoMCPServer(delay=args.delay)
    server.run()


if __name__ == "__main__":
    main()