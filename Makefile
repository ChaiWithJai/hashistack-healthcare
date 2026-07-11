.PHONY: run test check proof docker

run:
	cargo run

test:
	cargo test

check:
	cargo fmt --check
	cargo clippy --all-targets -- -D warnings
	cargo test

proof:
	@echo "Reviewer proof checklist"
	@echo "1. docs/product-use-case.md names customer workflow, managed default, and Rust boundary."
	@echo "2. docs/evidence-index.md links command output, CI, or screenshots."
	@echo "3. docs/ops-runbook.md explains how to run and inspect the service."
	@echo "4. docs/capstone-case-study.md names limitations before capability claims."

docker:
	docker compose up --build
