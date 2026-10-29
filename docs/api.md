# API guide

The full contract is `GET /openapi.json` (browsable at `/docs`). A test checks that every operation in that document is routed, so it cannot silently drift. This page covers conventions and a worked example.

## Conventions

- **Base URL** `http://127.0.0.1:8080` (merchant node) in the demo stack; the payer node is on `8081`.
- **Auth** `Authorization: Bearer sfd_test_…`. Keys belong to a project and carry scopes (`invoices:read`, `invoices:write`, `payments:read`, `payments:write`, `bitcoin:read`, `bitcoin:write`, `events:read`, `webhooks:write`, `sandbox:write`). Operator endpoints use `X-Admin-Token` instead.
- **Units** Amounts are integers. `amount_msat` is millisatoshis, `amount_sat` is satoshis, sandbox fiat uses `minor_units`. Floats are rejected.
- **Idempotency** `Idempotency-Key` is optional for invoices and **required** for payments and sandbox payouts. Repeating a request with the same key returns the stored resource (`200` instead of `201`). Reusing a key with different parameters returns `409 idempotency_key_reused`.
- **Errors** Always `{"error": {"code", "message", "request_id"}}`. `request_id` is also in the `X-Request-Id` header. Messages never contain SQL or node internals.
- **Pagination** List endpoints take `limit` (1–100, default 20) and `starting_after`; responses carry `has_more` and `next_cursor`. `/v1/events` uses `after=<seq>` and `next_after`.
- **Status vocabulary**
  - invoice: `open → paid | expired`
  - payment: `created → pending → succeeded | failed`, and `pending → unknown → succeeded | failed`. `unknown` is an ambiguous outcome: do not retry blindly, call `POST /v1/lightning/payments/{id}/reconcile` or wait.
  - on-chain: `unconfirmed → confirming → confirmed`
- **Real vs simulated** Resources carry `real_regtest_lightning` and `simulated` / `simulated_fiat` flags. `/v1/sandbox/*` is always simulated. `/v1/balances` returns real ledger totals and simulated balances in separate sections.
- **Timeouts** A client timeout on `POST /v1/lightning/payments` is not proof that nothing was sent. Retry with the same `Idempotency-Key` to read the payment.

## Webhooks

Register with `POST /v1/webhook-endpoints {"url": "…"}`. The response includes the signing secret **once**. Each delivery is a `POST` with:

```
Safrod-Signature: t=1761738000,v1=<hex hmac-sha256 of "{t}.{raw body}">
Safrod-Event-Id: evt_…
Safrod-Delivery-Id: dlv_…
```

Verify with the raw request body, a constant-time comparison, and reject timestamps more than five minutes old. Delivery is at least once with exponential backoff (5 s doubling, up to 8 attempts); deduplicate on `Safrod-Event-Id`. `GET /v1/webhook-deliveries` shows attempts and `POST /v1/webhook-deliveries/{id}/replay` re-queues one. Event types: `invoice.paid`, `invoice.expired`, `payment.succeeded`, `payment.failed`, `payment.unknown`, `bitcoin.transaction.detected|updated|confirmed`, `sandbox.payout.*`, `webhook.test`.

Webhook URLs must be public https in normal operation. The demo stack enables `SAFROD_WEBHOOK_ALLOW_PRIVATE` so the example app on a private address can receive them.

## Example

```bash
export KEY=sfd_test_…   # printed by scripts/setup.sh into .demo/keys.env

# Create an invoice for 2,500 sats (idempotent per key).
curl -s -X POST http://127.0.0.1:8080/v1/lightning/invoices \
  -H "authorization: Bearer $KEY" -H 'Idempotency-Key: order-1001' \
  -d '{"amount_sat": 2500, "description": "coffee"}'
```

```json
{
  "object": "lightning_invoice",
  "id": "inv_3a5fa0a2b94e428cb30a0a9b40244230",
  "status": "open",
  "amount_msat": 2500000,
  "bolt11": "lnbcrt25u1p…",
  "payment_hash": "1b6ce9a5…",
  "expires_at": "2026-10-29T22:40:41Z",
  "real_regtest_lightning": true,
  "simulated": false
}
```

```bash
# Pay it from another node.
curl -s -X POST http://127.0.0.1:8081/v1/lightning/payments \
  -H "authorization: Bearer $PAYER_KEY" -H 'Idempotency-Key: pay-1001' \
  -d '{"bolt11": "lnbcrt25u1p…"}'
```

```json
{ "object": "lightning_payment", "id": "pay_b0dc9aad…", "status": "pending", "amount_msat": 2500000 }
```

Poll `GET /v1/lightning/payments/{id}` or wait for `payment.succeeded`. On regtest the succeeded payment includes the `preimage`; `sha256(preimage)` equals `payment_hash`.

## Other examples

`scripts/demo-*.sh` are runnable curl examples of every flow, and `examples/merchant/server.js` is a TypeScript-free Node client showing order creation and webhook verification. A generated typed client is deferred until the endpoints stabilise.
