.PHONY: run test check proof docker staging

run:
	cargo run

test:
	cargo test

check:
	cargo fmt --check
	cargo clippy --all-targets -- -D warnings
	cargo test

# Virtual staging: boot the control plane and pressure-test the entire
# describe→audit workflow over HTTP. Pass BASE=http://host:port to target an
# already-running deployment instead. TODO(#2): grow into the full HashiStack
# staging (Nomad dev agent + Vault dev server + Postgres).
staging:
	./scripts/pressure-test.sh $(BASE)

proof:
	@echo "Reviewer proof checklist"
	@echo "1. docs/product-use-case.md names customer workflow, managed default, and Rust boundary."
	@echo "2. docs/evidence-index.md links command output, CI, or screenshots."
	@echo "3. docs/ops-runbook.md explains how to run and inspect the service."
	@echo "4. docs/capstone-case-study.md names limitations before capability claims."
	@echo "5. docs/GOAL.md states the end-user goal and the bar; scripts/pressure-test.sh proves the workflow without manual smoke testing."

docker:
	docker compose up --build
