.PHONY: demo-logos-core demo-logos-core-real test

# Run the Logos Core e2e demo (uses stub liblogos_core by default).
# The demo build.rs auto-compiles the C stub if LOGOS_CORE_LIB_DIR is not set.
demo-logos-core:
	cargo run -p logos-core-e2e-demo

# Run with the real Logos Core SDK (requires delivery_module + storage_module plugins).
# Example: LOGOS_CORE_LIB_DIR=/home/jimmy/logoscore-test make demo-logos-core-real
demo-logos-core-real:
	LD_LIBRARY_PATH="$(LOGOS_CORE_LIB_DIR):$$LD_LIBRARY_PATH" \
		LOGOS_CORE_LIB_DIR=$(LOGOS_CORE_LIB_DIR) \
		cargo run -p logos-core-e2e-demo

test:
	cargo test --workspace
