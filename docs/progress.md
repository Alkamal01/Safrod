# Progress

Last updated 2026-10-29. This file separates what was run from what was only written.

## Completed and tested

| Item | How it was verified |
| --- | --- |
| Domain types, state machines, API keys, webhook signing | 19 unit tests in `safrod-core`, including a fixed signature vector computed with openssl and checked again in Node |
| PostgreSQL schema, repositories, transactional outbox, idempotency, node-event dedupe | 9 integration tests against a real PostgreSQL 17 (`sqlx::test`) |
| REST API: auth and scopes, idempotent invoices and payments, ambiguity handling, pagination, SSRF checks, sandbox scenarios, OpenAPI-vs-router check, dashboard CSP | 17 integration tests against PostgreSQL with the in-memory Lightning mock |
| Webhook delivery: signing, retry with backoff, give-up, SSRF refusal | 3 delivery tests against a local HTTP receiver, 6 unit tests |
| Bitcoin Core RPC amount parsing and watcher logic | 3 unit tests; watcher exercised for real by Demo 2 |
| Daemon configuration safety rules | 7 unit tests |
| Example merchant app signature and replay handling | 4 Node tests |
| **Real regtest Lightning**: invoice, payment between two independent LDK Node processes, `PaymentReceived`/`PaymentSuccessful` handling, signed webhook, preimage proof | `scripts/demo-lightning.sh` against Bitcoin Core 29.2 regtest, two daemons, two databases |
| **Real on-chain flow**: address, deposit seen in mempool, 1 to 3 confirmations, confirmed event and webhook | `scripts/demo-onchain.sh` |
| Failure and exhausted-liquidity payment (`no_route`), idempotent retry | `scripts/demo-lightning.sh` steps 7 and 8 |
| Sandbox: failure refund, timeout then reconcile, success with real regtest settlement | `scripts/demo-sandbox.sh` |
| Restart recovery after `kill -9` of the merchant node | `scripts/demo-recovery.sh` |
| Clean-state setup | `scripts/dev-native.sh reset && start && scripts/setup.sh` |
| `cargo fmt`, `cargo clippy --workspace --all-targets --all-features -D warnings` | run locally, clean |
| Preimages absent from node logs at info level | grep of all logs after a payment |

## Implemented but unverified

- **Docker Compose stack and Dockerfiles.** `docker compose config` validates, but the images could not be pulled in the environment where this was built, so `make up` has not been run. The native runner exercises the same services and scripts.
- **CI.** The workflow is written. It has not run on GitHub. Its regtest job runs the same commands verified locally. `cargo deny` has not been run locally (not installed); the license allow-list was derived from `cargo metadata`.
- **Real in-flight crash recovery.** Reconciliation after a restart is tested with the mock backend and PostgreSQL, and a real node restart is tested for incoming payments. A real crash while an outgoing HTLC is in flight was not reproduced.
- Channel closure handling is only logged; nothing reacts to it.

## Blocked

- Docker image pulls from Docker Hub failed or timed out here (`docker pull postgres:16-alpine`: `context deadline exceeded`). Corrective action: run `make up` on a machine with registry access and record the result here.

## Known gaps

See `docs/threat-model.md`. Notably: no per-project spending limits, no TLS or rate limiting, webhook secrets stored in clear, no reorg reversal for on-chain records, no encryption of the LDK seed at rest.

## Next

1. Run and fix the Docker Compose path.
2. Per-project payment limits and API rate limiting.
3. Encrypt webhook secrets with an operator master key.
4. Reorg handling in the deposit watcher.
5. Typed client generated from the OpenAPI document; optional `grid-compat` adapter with a coverage table.
6. Evaluate Spark and UMA integrations behind disabled-by-default flags.
