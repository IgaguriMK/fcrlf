.PHONY: check
check:
	cargo test
	cargo fmt -- --check
	cargo clippy -- -D warnings
	cargo audit

.PHONY: min-check
min-check:
	cargo +nightly update -Z minimal-versions
	cargo check
