# Contributing to Safro

Thank you for contributing. Safro handles settlement logic, so correctness, clear failure behaviour, and reviewability take priority over feature count.

## Local checks

Run these commands before opening a pull request:

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all
```

## Pull requests

Keep each pull request focused and explain the settlement behaviour it changes. Changes that affect state transitions, persistence, quote verification, attestations, idempotency, Lightning release, or fiat payouts should include tests for success and failure paths.

Do not add credentials, signing keys, provider secrets, or customer data to the repository. Use development fixtures only for local testing.

## Design principles

- Never release Lightning based on an unauthenticated or unauthorized payout claim.
- Treat an ambiguous payout outcome as unknown until it is reconciled.
- Make external requests idempotent.
- Validate a state transition before performing its side effect.
- Preserve enough durable evidence to recover safely after a restart.

For security-sensitive reports, use the process in [SECURITY.md](SECURITY.md) instead of filing a public issue.
