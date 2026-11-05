.PHONY: dev

# Run x8ai from source, in this terminal. It leaves a background x8ai from this
# build running: `cargo run -p x8ai -- --stop` stops it.
dev:
	cargo run -p x8ai
