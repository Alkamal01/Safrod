# Upstream audit

Reference repositories are cloned into `references/` (ignored by Git) for reading only. Safrod consumes upstream code as published crate dependencies and never copies or patches it. Commits below are the exact checkouts inspected on 2026-10-29.

| Repository | Commit | License (from checked-out files) | How Safrod uses it |
| --- | --- | --- | --- |
| [lightningdevkit/rust-lightning](https://github.com/lightningdevkit/rust-lightning) | `caed8b9f86423775d61c17627410a63512d98a0c` | MIT OR Apache-2.0 (`LICENSE-MIT`, `LICENSE-APACHE`) | Transitive, through LDK Node. Read for event semantics and payment-failure reasons. |
| [lightningdevkit/ldk-node](https://github.com/lightningdevkit/ldk-node) | `5752ce26faf0420c01bac9859101060052d84d75` (main, 0.8.0+git) | MIT OR Apache-2.0 | Direct dependency, pinned to the published `0.7.1` release. |
| [bitcoindevkit/bdk](https://github.com/bitcoindevkit/bdk) | `f2528da88b1041bdddf547d677511aea19ad3573` | MIT OR Apache-2.0 | Transitive, through LDK Node's on-chain wallet. Not a direct dependency. |
| [buildonspark/spark](https://github.com/buildonspark/spark) | `0b3a32a05c9ac06cc411683551dd1f1bde9d0caa` | Apache-2.0 (`LICENSE`) | Research only. Go server and JS SDK; no Rust crate to depend on. |
| [uma-universal-money-address/protocol](https://github.com/uma-universal-money-address/protocol) | `7bc4e2418fe3df1ac111421451dcdc56dfd1707e` | Apache-2.0 | Research only. Specification documents (UMAD-01…11); an LNURL extension. |
| [lightsparkdev/grid-api](https://github.com/lightsparkdev/grid-api) | `e78e29b6030b25284665b6bdd8b8e437a0aa4d27` | Apache-2.0 | API design reference only. |

Bitcoin Core 29.2 (`bitcoin-29.2-x86_64-linux-gnu.tar.gz`, SHA-256 `1fd58d0a…f9009`, verified against the published `SHA256SUMS`) is used as a binary for regtest; it is not modified or redistributed.

## LDK Node

- A ready-made Lightning node built on `rust-lightning` and BDK. The `Builder` wires up persistence, networking, the on-chain wallet and chain sync, and exposes `bolt11_payment()`, `onchain_payment()`, `list_channels()`, `list_balances()`, `list_payments()` and an `Event` stream.
- Chain sources: Esplora, Electrum, Bitcoin Core RPC and REST (`set_chain_source_bitcoind_rpc`). Safrod uses Bitcoin Core RPC so one regtest `bitcoind` serves both nodes and the on-chain watcher.
- Events relevant to Safrod: `PaymentReceived`, `PaymentSuccessful`, `PaymentFailed`, `ChannelPending`, `ChannelReady`, `ChannelClosed`. Events must be acknowledged with `event_handled()`; unacknowledged events are redelivered after a restart, so handlers must be idempotent.
- Restrictions: `main` has moved ahead of the published release (`build` takes a `NodeEntropy` on `main`). Safrod pins the published version and follows its API, not `main`. Check `docs/progress.md` for the version actually built.
- MSRV on `main` is 1.85.

## BDK

BDK is consumed indirectly. LDK Node owns the on-chain wallet and its keys, so Safrod asks it for receive addresses and balances rather than running a second wallet. A separate `safrod-bitcoin` wallet would duplicate key custody for no demo benefit.

## Bitcoin Core

On-chain deposit tracking uses Bitcoin Core JSON-RPC (`getblockchaininfo`, `scantxoutset`, `generatetoaddress`). `scantxoutset` finds unspent outputs for an address without importing it into a Core wallet, which keeps Core free of keys. It scans the UTXO set, which is fine on regtest and would need replacing (descriptor wallet or an indexer) for larger networks.

## Spark

Spark is a Bitcoin layer-2 with a Go operator implementation and a JavaScript SDK. There is no Rust client in the checkout and the project is operated as a federated service. It is out of scope for the core; any adapter would be an optional, disabled-by-default module.

## UMA

UMA is a set of specification documents extending LNURL-pay with compliance data and currency conversion hints. The protocol repo ships schemas and spec text; SDKs live in separate repositories. Safrod does not implement UMA. The sandbox quote shape is informed by UMAD-06 but makes no compatibility claim.

## Grid API

The checked-out repository is documentation only: an OpenAPI source tree plus Mintlify docs (`CLAUDE.md` states "Documentation-only repo (no application code)"). The bundled `openapi.yaml` is generated and is not in the tree; conventions below come from the README and the repository's lint configuration. It is **not** a self-hostable Grid backend.

Conventions Safrod adopts (independently implemented, not copied):

- resource-oriented routes with separate quote and transaction resources;
- client-visible statuses and typed webhook events;
- webhook signature header with verification guidance;
- a sandbox environment for integrators.

Differences: Grid signs webhooks with ECDSA P-256 over the body; Safrod uses HMAC-SHA256 over `timestamp.body` with a per-endpoint secret. Safrod uses scoped bearer API keys rather than HTTP Basic. Safrod makes **no** Grid compatibility claim; an optional `grid-compat` adapter would need its own coverage table and tests.

## Licensing decision

Safrod is licensed `MIT OR Apache-2.0`, matching the Rust and Bitcoin ecosystem and compatible with every dependency inspected above (all MIT, Apache-2.0, or dual). Full license texts are in `LICENSE-MIT` and `LICENSE-APACHE`. `cargo deny` checks dependency licenses in CI.
