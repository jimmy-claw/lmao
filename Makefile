.PHONY: build test clippy fmt check doc clean examples bench demo-logos-core demo-logos-core-real

# Build all crates
build:
	~/.cargo/bin/cargo build --workspace

# Build in release mode
release:
	~/.cargo/bin/cargo build --workspace --release

# Run all tests
test:
	~/.cargo/bin/cargo test --workspace

# Run clippy lints
clippy:
	~/.cargo/bin/cargo clippy --workspace -- -D warnings

# Format code
fmt:
	~/.cargo/bin/cargo fmt --all

# Check formatting (CI mode)
fmt-check:
	~/.cargo/bin/cargo fmt --all -- --check

# Full CI check (format + clippy + test)
check: fmt-check clippy test

# Generate documentation
doc:
	~/.cargo/bin/cargo doc --workspace --no-deps

# Build examples
examples:
	~/.cargo/bin/cargo build --examples

# Run benchmarks (requires criterion, see jimmy/add-benchmarks branch)
bench:
	~/.cargo/bin/cargo bench --workspace

# Run the two-agent demo
demo:
	~/.cargo/bin/cargo run --example two_agents

# Run the ping-pong demo (optionally encrypted)
demo-ping:
	~/.cargo/bin/cargo run --example ping_pong

demo-ping-encrypted:
	~/.cargo/bin/cargo run --example ping_pong -- --encrypt

# Run the echo agent
demo-echo:
	~/.cargo/bin/cargo run --example echo_agent

# Logos Core e2e demo (stub)
demo-logos-core:
	~/.cargo/bin/cargo run -p logos-core-e2e-demo

# Logos Core e2e demo (real SDK)
# Usage: LOGOS_CORE_LIB_DIR=/path/to/logoscore make demo-logos-core-real
demo-logos-core-real:
	LD_LIBRARY_PATH="$(LOGOS_CORE_LIB_DIR):$$LD_LIBRARY_PATH" \
		LOGOS_CORE_LIB_DIR=$(LOGOS_CORE_LIB_DIR) \
		~/.cargo/bin/cargo run -p logos-core-e2e-demo

# Build MCP bridge
mcp:
	~/.cargo/bin/cargo build -p logos-messaging-a2a-mcp --release

# Build CLI
cli:
	~/.cargo/bin/cargo build -p logos-messaging-a2a-cli --release

# Build FFI shared library
ffi:
	~/.cargo/bin/cargo build -p logos-messaging-a2a-ffi --release

# Clean build artifacts
clean:
	~/.cargo/bin/cargo clean
