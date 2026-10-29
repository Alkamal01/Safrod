# Threat model

Safrod is an unaudited prototype for Bitcoin **regtest**. This document records what it defends against, what it does not, and the assumptions that make the defences meaningful. It is not a security certification.

## Assets

| Asset | Where it lives |
| --- | --- |
| Lightning channel funds and on-chain wallet seed | LDK Node data directory (`SAFROD_LDK_STORAGE_DIR`), `keys_seed` file |
| Payment records, invoices, ledger | PostgreSQL |
| Project API keys | PostgreSQL, as SHA-256 digests only |
| Webhook signing secrets | PostgreSQL, in clear (see limits) |
| Operator token | `SAFROD_ADMIN_TOKEN` environment variable |

## Trust assumptions

- The **operator** is trusted. They hold the node's keys and the admin token. Safrod is a self-hosted service, **not** a self-custodial wallet for end users, and it makes no custody claim beyond "the operator holds the keys".
- PostgreSQL and the host are trusted and access-controlled. Anyone with database write access can alter the ledger.
- Bitcoin Core is a single trusted node. Safrod does not cross-check it.
- A project API key with `payments:write` can spend from the **shared node wallet**. Project ledgers are accounting views over one pool of liquidity; there are no per-project spending limits.

## Threats and mitigations

| Threat | Mitigation | Tested |
| --- | --- | --- |
| Unauthenticated API access | Bearer keys, hashed at rest, scopes per operation; separate operator token compared in constant time | API tests: missing, bogus, under-scoped keys |
| Admin API exposed by accident | Disabled unless `SAFROD_ADMIN_TOKEN` is set (24+ chars); dev endpoints are off by default and refused outside regtest; daemon refuses non-loopback bind without `SAFROD_ALLOW_PUBLIC_BIND` | config unit tests; API test for dev endpoints |
| Duplicate payments from client retries or timeouts | Mandatory `Idempotency-Key`; stored request hash detects key reuse with different parameters; `pending` is persisted before the node is asked to send; LDK Node also refuses to pay one invoice twice | API tests; storage tests |
| Ambiguous outcome treated as failure (double pay on retry) | `unknown` status; reconciliation never converts a missing node record into `failed` for a payment that was already `pending` | API test: restart reconciliation |
| Replayed or duplicated node events | Events deduplicated by key in the transaction that applies them; final payment states cannot change | storage and API tests |
| Out-of-order or late events | State machines reject transitions out of a final state | core and storage tests |
| Forged webhooks | HMAC-SHA256 over `timestamp.body`, constant-time verification, 5 minute tolerance on the receiver | core tests; fixed cross-language vector; merchant app tests |
| Webhook SSRF (internal addresses, metadata endpoints, DNS rebinding, redirects) | URL checked at registration and again on the resolved address at each delivery; connection pinned to the checked address; redirects disabled; credentials in URLs refused; https required unless development mode | url_guard unit tests; delivery integration tests |
| Wrong-network invoice | Invoice prefix and parse checked against the node network before any record is created | API tests |
| Float rounding of money | Integer satoshis and millisatoshis; Bitcoin Core amounts parsed from decimal strings | core and bitcoin tests |
| Secret leakage via logs and errors | Client errors are fixed messages with a request id; preimages are not logged by Safrod, and LDK's file log is set to info level, which does not contain them (checked with grep after a payment); API keys are never logged | manual check, see `docs/progress.md` |
| Mainnet use of unaudited code | `SAFROD_NETWORK=bitcoin` is refused at startup | config unit test |

## Known limits (not defended)

- **Keys at rest.** The LDK seed is a plain file (mode 0400 in a 0700 directory). There is no encryption at rest, HSM, or key rotation. Webhook signing secrets are stored in clear because they are needed to sign; a database dump reveals them. Encrypting them with a master key is planned.
- **No TLS.** The daemon serves plain HTTP and expects a reverse proxy for TLS. No mutual TLS.
- **No rate limiting** or request quotas.
- **Shared node liquidity** (above). Per-project limits are planned.
- **Crash between node and database.** If the process dies after the node created an invoice but before it was stored, a later payment of that invoice is logged as an unknown payment hash and dropped. Funds arrive at the node but are not credited to a project.
- **On-chain watcher.** It finds outputs through `scantxoutset` (confirmed UTXOs only) plus a mempool scan. A deposit that is later spent disappears from the scan and keeps its last recorded confirmation count. A reorg that removes a deposit is not reverted in the ledger. The scan is fine on regtest and would not scale to mainnet-sized UTXO sets.
- **LDK Node data.** `ldk_node.log` and the SQLite store in the data directory contain payment hashes and channel state; protect the directory and exclude it from log shipping.
- **Single operator token** with no rotation or audit trail.
- **No audit.** Neither Safrod nor this threat model has had external review.

## Not claimed

Safrod does not provide fiat payout, FX execution, KYC/AML, stable-value guarantees, atomic fiat settlement, or mainnet support. Sandbox endpoints are simulations and are labelled so in every response. A valid signature proves who made an assertion, not that an off-chain payout happened.
