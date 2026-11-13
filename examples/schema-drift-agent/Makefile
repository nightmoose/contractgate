# One command: build the DataContractGate CLI if needed, install dbt-duckdb
# into a local venv, and run catch -> impact -> draft.
REPO_ROOT := $(abspath $(dir $(lastword $(MAKEFILE_LIST)))/../..)
CLI := $(REPO_ROOT)/target/debug/contractgate
export SQLX_OFFLINE := true

.PHONY: demo cli venv

demo: venv cli
	CONTRACTGATE_BIN="$(CLI)" .venv/bin/python drift_agent.py

cli:
	SQLX_OFFLINE=true cargo build --manifest-path "$(REPO_ROOT)/Cargo.toml" --bin contractgate

venv: .venv/bin/dbt

.venv/bin/dbt: requirements.txt
	python3 -m venv .venv
	.venv/bin/pip install --upgrade pip
	.venv/bin/pip install -r requirements.txt
