# Contributing to Safrod

Safrod handles payment state, so correctness, clear failure behaviour and reviewability take priority over feature count.

## Local checks

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
DATABASE_URL=postgres://user@127.0.0.1:5432/postgres cargo test --workspace
(cd examples/merchant && node --test)
```

Database tests use `sqlx::test`, which creates a throwaway database per test, so the user needs `CREATEDB`. Changes that touch Lightning, Bitcoin Core, webhooks or the sandbox should also pass `scripts/dev-native.sh start && scripts/setup.sh && scripts/demo.sh all` (see [`docs/demo.md`](docs/demo.md)).

## Pull requests

Open an issue before large design changes. Keep each pull request focused and explain the behaviour it changes and how you tested it. Changes to state transitions, idempotency, event handling, persistence, webhook delivery or signing need tests for the success and failure paths. Report test results honestly: say which commands you ran and which you could not.

Do not add credentials, signing keys, provider secrets or customer data. Use the fake values in `.env.example`.

## Design principles

- Never infer settlement from an HTTP response; settle on a node event or a reconciled node record.
- Treat an ambiguous outcome as `unknown` until it is reconciled. A timeout is not a failure.
- Make external requests idempotent, and make event handlers safe to run twice.
- Validate a state transition before performing its side effect, and write a state change and its notification in one transaction.
- Use integer satoshis and millisatoshis; no floating-point money.
- Treat peer messages, user input, webhooks and external API responses as untrusted.
- Label simulated behaviour as simulated, everywhere it is visible.
- Do not claim a capability that was not run end to end.

## Code style

Prefer small, explicit types and ordinary control flow. Comments explain why, invariants and non-obvious protocol behaviour, not what a line does. Commit messages are specific and human, for example `lightning: persist settled invoice events`.

For security-sensitive reports, use [`SECURITY.md`](SECURITY.md) instead of a public issue.
