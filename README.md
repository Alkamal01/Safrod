# Safro

[![CI](https://github.com/Alkamal01/Safrod/actions/workflows/ci.yml/badge.svg)](https://github.com/Alkamal01/Safrod/actions/workflows/ci.yml)
[![Rust](https://img.shields.io/badge/Rust-stable-orange?logo=rust)](https://www.rust-lang.org/)
[![License: MIT OR Apache--2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](#license)

**Provider-to-provider settlement infrastructure for cross-border payments.**

Safro lets independent payment providers settle cross-border obligations over Bitcoin Lightning while they continue to collect and pay out through their own local rails. A source provider reserves local funds, a destination provider pays the beneficiary, and the providers settle the resulting obligation over Lightning.

[About](#about) · [Architecture](#settlement-flow) · [Run locally](#run-locally) · [API](#api-surface) · [Contributing](CONTRIBUTING.md) · [Security](SECURITY.md) · [License](#license)

## Status

Safro is an early reference implementation. Its settlement state machine, signed quotes, payout attestations, durable settlement records, peer API, and Core Lightning backend are implemented and covered by automated tests. It is not ready to process production customer funds: the daemon currently uses a deterministic local fiat adapter and does not yet provide a live fiat-rail integration or hardened provider transport.

## About

Safro is not a consumer wallet, a remittance app, or a replacement for local payment providers. Each provider retains its customer relationship, KYC/AML responsibilities, licensing, and local collection and payout rails.

`safrod` is the Rust reference daemon. It provides:

- a typed settlement state machine with explicit release, refund, unknown, and dispute states;
- signed payout attestations checked against a configured peer identity;
- signed executable quotes that bind provider identities, amounts, and expiry;
- an Axum JSON API for provider and peer actions;
- durable settlement records with restart recovery for accepted Lightning invoices;
- a Core Lightning JSON-RPC backend for BOLT11 invoice creation and payment; and
- a deterministic fiat adapter for local development and automated tests.

## Settlement flow

```mermaid
sequenceDiagram
    participant S as Source provider / Safro node
    participant D as Destination provider / Safro node
    participant R as Destination payout rail
    participant L as Lightning Network

    S->>D: Request destination preparation
    D->>D: Reserve destination liquidity
    D-->>S: BOLT11 invoice
    S->>S: Reserve source funds and record invoice
    S->>D: Request beneficiary payout
    D->>R: Initiate payout with idempotency key
    R-->>D: Confirmed, failed, or unknown
    D-->>S: Signed payout attestation
    alt confirmed
        S->>L: Pay destination invoice
    else failed
        S->>S: Refund source reservation
    else outcome unknown
        S->>D: Reconcile later
    end
```

Lightning is released only after a valid, authorized attestation confirms the destination payout. A timeout becomes `PayoutUnknown`; it is never treated as a failure automatically.

## Prerequisites

- Rust stable, including `rustfmt` and `clippy`
- Two reachable Core Lightning nodes with channel liquidity between them
- Access to each node's `lightning-rpc` Unix socket

The sample TOML files are local-development examples. They contain fixed demo identity keys and machine-specific socket and store paths. Copy them outside version control, replace every value, and protect production signing keys with a proper secret-management system.

## Run locally

Build and verify the daemon:

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all
cargo run -- configs/ngn-node.toml
```

Start the destination node in a second terminal:

```bash
cargo run -- configs/kes-node.toml
```

Both configurations expect their configured Core Lightning RPC sockets to exist. The NGN node listens on `127.0.0.1:8081`; the KES node listens on `127.0.0.1:8082`.

To create a local settlement through the source node:

```bash
curl -X POST http://127.0.0.1:8081/settlements \
  -H 'content-type: application/json' \
  -d '{
    "fiat_amount": {"currency":"Ngn", "minor_units":500000},
    "destination_amount": {"currency":"Kes", "minor_units":30000},
    "btc_amount_msat": 100000,
    "counterparty": "kes-provider",
    "beneficiary": "local-development-beneficiary"
  }'
```

The response contains a `settlement_id`. Request destination payout and apply the resulting attestation with:

```bash
curl -X POST http://127.0.0.1:8081/settlements/<settlement_id>/settle
```

Inspect progress with:

```bash
curl http://127.0.0.1:8081/settlements/<settlement_id>
```

## API surface

Provider-facing endpoints:

| Method | Path | Purpose |
| --- | --- | --- |
| `POST` | `/settlements` | Create a source-side settlement and obtain the destination invoice. |
| `GET` | `/settlements/:id` | Read the settlement state. |
| `POST` | `/settlements/:id/settle` | Ask the destination peer to pay out; release Lightning only after confirmation. |
| `POST` | `/settlements/:id/reconcile-peer` | Reconcile a destination payout that is still unknown. |
| `POST` | `/settlements/:id/release` | Release an already-confirmed source settlement. |
| `POST` | `/settlements/:id/refund` | Refund an already-failed source settlement. |

Peer-facing endpoints are under `/peer/`. They prepare a destination settlement, issue invoices, accept payout requests, reconcile payout status, and receive signed payout attestations. They are protocol endpoints and should be exposed only through authenticated, mutually authorized provider transport.

## States and failure handling

The normal success path is:

`Created → Quoted → SourceReserved → DestinationReserved → SettlementConditionCreated → PayoutRequested → PayoutConfirmed → SettlementReleased`

A definite destination failure follows `PayoutFailed → Refunded`. An ambiguous rail response follows `PayoutUnknown` until the destination provider reconciles it. Conflicting signed evidence moves the settlement to `Disputed` for human and contractual resolution.

## Production work still required

Before accepting real payments, the protocol needs the following operational and engineering work:

- replace the development fiat adapter with adapters for onboarded, regulated providers and verify provider webhook signatures;
- add mutual TLS or equivalent authenticated transport, request signing, replay protection, authorization, and rate limits to peer endpoints;
- keep signing keys outside TOML files, add key rotation, audit logging, and monitored backups for settlement records;
- complete quote exchange and destination-side quote verification on the peer API, then version the wire protocol; and
- establish corridor-specific legal agreements, safeguarding, sanctions/AML controls, reconciliation operations, incident response, and dispute rules.

## Development

The GitHub Actions workflow runs formatting, Clippy with warnings denied, and the full test suite for pushes and pull requests to `main`. See [CONTRIBUTING.md](CONTRIBUTING.md) for the local workflow.

## Security

Please do not open public issues for vulnerabilities involving funds, signing keys, settlement integrity, or authentication. Follow the private reporting process in [SECURITY.md](SECURITY.md).

## License

Safro is dual-licensed under MIT or Apache-2.0, at your option. The package manifest declares the license; the full license texts will be added before the first public release.
