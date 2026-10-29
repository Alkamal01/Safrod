# Demo guide

Everything runs on Bitcoin **regtest**: a private chain where blocks are mined on demand and coins have no value. Nothing here touches mainnet or real funds.

The stack is Bitcoin Core, PostgreSQL, **two independent Safrod nodes** (merchant on `:8080`, payer on `:8081`, each with its own database, keys and LDK Node) and the example merchant app on `:4000`.

## Start

Docker (the intended path; see the status note below):

```bash
make up        # build and start the stack (copies .env.example to .env)
make setup     # fund the payer, open a real channel, register the webhook
make demo      # run every demo
make reset     # stop everything and delete all regtest state and keys
```

Without Docker (what CI and the recorded runs below used). You need `bitcoind`, `node`, `psql`, a PostgreSQL user that may create databases, and a built daemon:

```bash
export BITCOIN_BIN=/path/to/bitcoin-29.x/bin
export SAFROD_DEV_PG_URL=postgres://safrod@127.0.0.1:5432
cargo build -p safrod
scripts/dev-native.sh start
scripts/setup.sh
scripts/demo.sh all
scripts/dev-native.sh reset      # clean slate
```

Open `http://127.0.0.1:8080/dashboard` and paste the merchant key from `.demo/keys.env` (`KA`) to watch invoices, payments, channels, events and webhook deliveries. Open `http://127.0.0.1:8080/flow-builder` to reach the adapted Next.js flow builder (served on port 3001). Choose a supported source and destination to view Safrod API calls. The builder generates cURL examples but does not execute payments; set `SAFROD_API_KEY` from `.demo/keys.env` before running them. Lightning and on-chain flows use regtest. NGN/KES examples are sandbox simulations. For the native stack, start the UI separately with `make flow-ui`. Open `http://127.0.0.1:4000` for the merchant app and `http://127.0.0.1:8080/docs` for the API reference. `.demo/` holds generated keys and is git-ignored.

## Demo 1: real Lightning (`scripts/demo-lightning.sh`)

merchant creates invoice → second node pays → LDK payment event → database updates → signed webhook → merchant app shows paid.

Expected output (abridged, from a native run):

```
== 1. Merchant app creates an order (real regtest Lightning invoice)
   invoice: inv_3a5fa0a2b94e428cb30a0a9b40244230
== 3. Payment outcome on the payer side
{"id":"pay_b0dc…","status":"succeeded","amount_msat":2500000,"fee_msat":0,"payment_hash":"1b6ce9a5…"}
== 4. Merchant ledger (written by the LDK PaymentReceived event)
{"id":"inv_3a5f…","status":"paid","amount_received_msat":2500000,…}
== 5. Signed webhook reaches the merchant app, which marks the order paid
{"id":"order_a44e391df0a1","status":"paid",…}
{"type":"invoice.paid","simulated":false,"outcome":"applied"}
== 6. Proof of payment (regtest only)
   matches the payment hash: the payment really settled
== 7. Failure case: an invoice larger than the channel can carry
{"id":"pay_3ea1…","status":"failed","failure_reason":"no_route"}
== 8. Idempotent retry …
   same payment returned: pay_b0dc…
== Done: Demo 1 passed
```

The script exits non-zero if any step does not hold, for example if `sha256(preimage)` differs from the payment hash.

## Demo 2: on-chain deposit (`scripts/demo-onchain.sh`)

merchant address → Bitcoin Core test wallet pays → blocks mined → confirmations update → events and webhooks.

```
== 3. Deposit detected in the mempool
{"amount_sat":50000,"status":"unconfirmed","confirmations":0}
== 4. Mining blocks; confirmations update
{"status":"confirming","confirmations":1}
{"status":"confirming","confirmations":2}
{"status":"confirmed","confirmations":3}
== 5. Merchant app shows the order paid (via signed webhook)
== Done: Demo 2 passed
```

## Demo 3: integration sandbox (`scripts/demo-sandbox.sh`)

> **The fiat side is simulated.** Quotes, NGN/KES balances and payout outcomes are synthetic and every response says `simulated_fiat: true`. No NGN or KES moves, no provider is contacted, rates are fixed constants. The only real part is the optional BTC settlement invoice, paid over regtest Lightning.

Three deterministic scenarios on a quote of 5,000.00 NGN → 425.70 KES (1 % simulated fee):

| Scenario | Simulated payout | Real Bitcoin | Simulated balance |
| --- | --- | --- | --- |
| `failure` | `failed` | not released (invoice stays `open`) | refunded |
| `timeout` | `unknown` until reconciled; then `succeeded` | released only after reconciliation | debited |
| `success` | `succeeded` | settlement invoice paid over regtest Lightning | debited |

A timeout does not prove the destination failed to pay out, so Bitcoin is never released on one. The script asserts that the settlement invoice is still `open` after `failure` and `timeout`.

## Restart recovery (`scripts/demo-recovery.sh`)

Creates an invoice, kills the merchant node with `kill -9`, restarts it, confirms the invoice survived and the channel reconnected, pays it, and asserts exactly one `invoice.paid` event. With Docker it uses `docker compose kill safrod`.

## Reset

`make reset` (Docker) or `scripts/dev-native.sh reset` (native) drops the databases, the regtest chain, node data and generated keys. `scripts/setup.sh` is idempotent and safe to re-run on a live stack.

## Status of these instructions

The native path was run end to end from a clean state on 2026-10-29 (outputs above are from that run). The Docker Compose path is written and its configuration validates with `docker compose config`, but it has **not been run**: image pulls were unavailable when it was written. See `docs/progress.md`.
