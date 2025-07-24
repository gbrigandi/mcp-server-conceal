# Python Integration Tests for mcp-server-conceal

This directory contains Python-based integration tests for the `mcp-server-conceal` project.

## Setup

First, ensure you have built the Rust binary and installed the necessary Python packages:

```bash
# 1. Build the release binary from the root directory
cargo build --release

# 2. Install Python dependencies
pip install -r requirements.txt
```

## Running Tests

You can run all tests or target specific files.

```bash
# Run all tests verbosely
pytest -v

# Run a specific test file
pytest test_comprehensive.py -v
```

## Test Suite

- `test_comprehensive.py`: Comprehensive end-to-end testing.
- `test_pseudo_anonymization.py`: Verifies the consistency of pseudo-anonymization.
- `test_model_configs.py`: Tests various Ollama model configurations.
- `benchmark_*.py`: Scripts for performance benchmarking.

## Core Components

- `conftest.py`: Shared test utilities and Pytest fixtures.
- `requirements.txt`: Python dependencies for the test suite.
