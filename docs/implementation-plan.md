# Implementation plan

Ordered, issue-sized work. Each item lands with tests.

1. **Workspace and core types** — money, ids, state machines, API keys, webhook signing.
2. **Storage** — migrations and repositories for projects, keys, invoices, payments, events, outbox.
3. **Lightning backend** — `LightningBackend` trait, LDK Node adapter against Bitcoin Core RPC.
4. **Regtest environment** — Bitcoin Core, two nodes, mining and channel-setup scripts.
5. **Invoices end to end** — create, receive via node event, persist, idempotent event handling.
6. **Outgoing payments** — idempotency, `pending`/`unknown`, restart reconciliation.
7. **On-chain deposits** — address generation and confirmation watcher.
8. **Webhooks** — outbox, signed delivery, retries, SSRF guard.
9. **API** — auth middleware, routes, OpenAPI, error shape, pagination.
10. **Sandbox** — simulated quotes and payouts with deterministic scenarios.
11. **Merchant example and dashboard**.
12. **Hardening and CI** — clippy, `cargo deny`, regtest integration job, threat model, docs.

Deferred: SDKs, `grid-compat` adapter, live fiat adapters, Spark and UMA experiments.
