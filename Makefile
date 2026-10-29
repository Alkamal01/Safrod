.PHONY: help check test fmt lint up setup demo down reset native-up native-down native-reset flow-ui

help:
	@echo "make check        fmt, clippy and tests (needs DATABASE_URL for the database tests)"
	@echo "make up           build and start the regtest stack in Docker"
	@echo "make setup        fund nodes, open a channel, register the webhook"
	@echo "make demo         run every demo against the running stack"
	@echo "make down         stop the stack"
	@echo "make reset        stop the stack and delete all regtest state and keys"
	@echo "make native-up    the same stack without Docker (see scripts/dev-native.sh)"
	@echo "make flow-ui      run the Safrod Next.js flow builder at http://127.0.0.1:3001"

fmt:
	cargo fmt --all -- --check

lint:
	cargo clippy --workspace --all-targets --all-features -- -D warnings

test:
	cargo test --workspace
	cd examples/merchant && node --test

check: fmt lint test

.env:
	cp .env.example .env

up: .env
	docker compose up --build -d --wait

setup:
	scripts/setup.sh

demo:
	scripts/demo.sh all

down:
	docker compose down

reset:
	docker compose down -v
	rm -rf .demo

native-up:
	scripts/dev-native.sh start

native-down:
	scripts/dev-native.sh stop

native-reset:
	scripts/dev-native.sh reset

flow-ui:
	cd dashboard/flow-builder-ui && npm install && npm run dev -- --hostname 127.0.0.1 --port 3001
