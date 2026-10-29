# ADR 0001: Lightning node implementation

Status: accepted

## Context

Safrod needs real Lightning on regtest between two independently controlled nodes, with invoice creation, payments, events and channel visibility, embedded in a Rust service.

## Options

1. **LDK Node** — batteries-included node library on `rust-lightning` and BDK.
2. **`rust-lightning` directly** — maximum control; the application must supply a chain interface, persistence, a background processor, a network graph, routing, a wallet and key management.
3. **External node (Core Lightning, LND) behind an adapter** — the earlier Safrod prototype used Core Lightning. Mature, but adds a second runtime and process per node and is not "LDK-first".

## Decision

Use **LDK Node**, pinned to a published release, with Bitcoin Core RPC as the chain source.

## Trade-offs

- Gains: a small, auditable adapter; wallet, persistence, chain sync and event handling already integrated; each demo node is one process.
- Costs: less control over channel and routing policy; API changes between releases (pinned); a node's wallet keys live with the process, so Safrod is **not** self-custodial in any sense beyond "the operator holds the keys".
- Escape hatch: the Lightning surface is behind the `LightningBackend` trait in `safrod-lightning`, so a lower-level or external backend can replace it without touching the payment engine.
- The Core Lightning settlement prototype is kept in `legacy/settlement` and is not part of the new service.
