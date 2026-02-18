# Load environment variables from .env file
ifneq (,$(wildcard ./.env))
    include .env
    export $(shell sed 's/=.*//' .env)
else
    $(warning .env file not found. Environment variables not loaded.)
endif

# Variables
PROJ_REPO = github.com/habedi/template-rust-project
BINARY_NAME := $(or $(PROJ_BINARY), $(notdir $(PROJ_REPO)))
BINARY := target/release/$(BINARY_NAME)
PATH := /snap/bin:$(PATH)
DEBUG_PROJ := 0
RUST_BACKTRACE := 1
ASSET_DIR := assets
TEST_DATA_DIR := tests/testdata
SHELL := /bin/bash

# Default target
.DEFAULT_GOAL := help

.PHONY: help
help: ## Show help messages for all available targets
	@grep -E '^[a-zA-Z_-]+:.*## .*$$' Makefile | \
	awk 'BEGIN {FS = ":.*## "}; {printf "\033[36m%-30s\033[0m %s\n", $$1, $$2}'

.PHONY: format
format: ## Format Rust files
	@echo "Formatting Rust files..."
	@cargo fmt

.PHONY: test
test: format ## Run the tests
	@echo "Running tests..."
	@DEBUG_PROJ=$(DEBUG_PROJ) RUST_BACKTRACE=$(RUST_BACKTRACE) cargo test --all-targets --workspace -- --nocapture

.PHONY: coverage
coverage: format ## Generate test coverage report
	@echo "Generating test coverage report..."
	@DEBUG_PROJ=$(DEBUG_PROJ) cargo tarpaulin --out Xml --out Html

.PHONY: build
build: format ## Build the binary for the current platform
	@echo "Building the project..."
	@DEBUG_PROJ=$(DEBUG_PROJ) cargo build --release

.PHONY: run
run: build ## Build and run the binary
	@echo "Running the $(BINARY) binary..."
	@DEBUG_PROJ=$(DEBUG_PROJ) ./$(BINARY)

.PHONY: clean
clean: ## Remove generated and temporary files
	@echo "Cleaning up..."
	@cargo clean
	@rm -f $(ASSET_DIR)/*.svg && echo "Removed SVG files; might want to run 'make figs' to regenerate them."

.PHONY: install-snap
install-snap: ## Install a few dependencies using Snapcraft
	@echo "Installing the snap package..."
	@sudo apt-get update
	@sudo apt-get install -y snapd graphviz wget
	@sudo snap refresh
	@sudo snap install rustup --classic

.PHONY: install-deps
install-deps: install-snap ## Install development dependencies
	@echo "Installing dependencies..."
	@rustup component add rustfmt clippy
	@cargo install cargo-tarpaulin
	@cargo install cargo-audit
	@cargo install cargo-careful
	@cargo install cargo-nextest

.PHONY: lint
lint: format ## Run the linters
	@echo "Linting Rust files..."
	@DEBUG_PROJ=$(DEBUG_PROJ) cargo clippy -- -D warnings -D clippy::unwrap_used -D clippy::expect_used

.PHONY: publish
publish: ## Publish the package to crates.io (requires CARGO_REGISTRY_TOKEN to be set)
	@echo "Publishing the package to Cargo registry..."
	@cargo publish --token $(CARGO_REGISTRY_TOKEN)

.PHONY: bench
bench: ## Run the benchmarks
	@echo "Running benchmarks..."
	@DEBUG_PROJ=$(DEBUG_PROJ) cargo bench

.PHONY: audit
audit: ## Run security audit on Rust dependencies
	@echo "Running security audit..."
	@cargo audit

.PHONY: rust-careful
careful: ## Run security checks on Rust code
	@echo "Running security checks..."
	@cargo careful

.PHONY: docs
docs: format ## Generate the documentation
	@echo "Generating documentation..."
	@cargo doc --no-deps --document-private-items

.PHONE: figs
figs: ## Generate the figures in the assets directory
	@echo "Generating figures..."
	@$(SHELL) $(ASSET_DIR)/make_figures.sh $(ASSET_DIR)

.PHONY: fix-lint
fix-lint: ## Fix the linter warnings
	@echo "Fixing linter warnings..."
	@cargo clippy --fix --allow-dirty --allow-staged --all-targets --workspace --all-features -- -D warnings -D clippy::unwrap_used -D clippy::expect_used

.PHONY: testdata
testdata: ## Download the datasets used in tests
	@echo "Downloading test data..."
	@$(SHELL) $(TEST_DATA_DIR)/download_datasets.sh $(TEST_DATA_DIR)

.PHONY: nextest
nextest: ## Run tests using nextest
	@echo "Running tests using nextest..."
	@DEBUG_PROJ=$(DEBUG_PROJ) RUST_BACKTRACE=$(RUST_BACKTRACE) cargo nextest run

.PHONY: setup-hooks
setup-hooks: ## Install Git hooks (pre-commit and pre-push)
	@echo "Setting up Git hooks..."
	@if ! command -v pre-commit &> /dev/null; then \
	   echo "pre-commit not found. Please install it using 'pip install pre-commit'"; \
	   exit 1; \
	fi
	@pre-commit install --hook-type pre-commit
	@pre-commit install --hook-type pre-push
	@pre-commit install-hooks

.PHONY: test-hooks
test-hooks: ## Test Git hooks on all files
	@echo "Testing Git hooks..."
	@pre-commit run --all-files --show-diff-on-failure
