# Makefile for Ethera Spec
# Run `make help` to see available commands

GREEN := \033[0;32m
YELLOW := \033[0;33m
BLUE := \033[0;34m
NC := \033[0m

.PHONY: all help build test lint fmt fmt-fix clean check verify deny

all: verify

help: ## Display this help message
	@echo "$(BLUE)Ethera Spec$(NC)"
	@echo ""
	@echo "$(YELLOW)Targets:$(NC)"
	@grep -E '^[a-zA-Z_-]+:.*?## .*$$' $(MAKEFILE_LIST) | sort | awk 'BEGIN {FS = ":.*?## "}; {printf "  $(GREEN)%-20s$(NC) %s\n", $$1, $$2}'

build: ## Build the workspace
	@cargo build --workspace

clean: ## Clean build artifacts
	@cargo clean

test: ## Run all tests
	@cargo test --workspace

lint: ## Run clippy
	@cargo clippy --workspace --all-targets -- -D warnings

fmt: ## Check formatting
	@cargo fmt --all -- --check

fmt-fix: ## Fix formatting
	@cargo fmt --all

deny: ## Run cargo-deny license and advisory checks
	@cargo deny check

check: lint test ## Run linting and tests

verify: fmt lint test build ## Full verification
