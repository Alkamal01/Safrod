#!/usr/bin/env bash
# Demo 1, real Lightning: merchant invoice -> second node pays -> payment event
# -> database update -> signed webhook -> merchant app shows paid.
source "$(dirname "${BASH_SOURCE[0]}")/lib.sh"
load_keys
RUN="$(date +%s)"

say "1. Merchant app creates an order (real regtest Lightning invoice)"
order="$(curl -sS -X POST "$MERCHANT_APP/api/orders" -d '{"kind":"lightning","amount_sat":2500}')"
oid="$(jq -r .id <<<"$order")"; invoice_id="$(jq -r .invoice_id <<<"$order")"
bolt11="$(jq -r .bolt11 <<<"$order")"
[ "$invoice_id" != null ] || die "order creation failed: $order"
note "order:   $oid"
note "invoice: $invoice_id"
note "bolt11:  ${bolt11:0:60}..."

say "2. The payer node pays it (separate process, database and keys)"
pay="$(api POST "$PAYER_API" /v1/lightning/payments "$KB" "{\"bolt11\":\"$bolt11\"}" "pay-$RUN-ok")"
pid="$(jq -r .id <<<"$pay")"
note "payment $pid -> $(jq -r .status <<<"$pay")"

say "3. Payment outcome on the payer side"
final="$(poll_json "payment to settle" 30 '.status == "succeeded" or .status == "failed"' api GET "$PAYER_API" "/v1/lightning/payments/$pid" "$KB")"
jq -c '{id, status, amount_msat, fee_msat, payment_hash}' <<<"$final"
[ "$(jq -r .status <<<"$final")" = succeeded ] || die "payment did not succeed: $final"

say "4. Merchant ledger (written by the LDK PaymentReceived event)"
inv="$(poll_json "invoice to be paid" 30 '.status == "paid"' api GET "$MERCHANT_API" "/v1/lightning/invoices/$invoice_id" "$KA")"
jq -c '{id, status, amount_received_msat, paid_at}' <<<"$inv"

say "5. Signed webhook reaches the merchant app, which marks the order paid"
paid="$(poll_json "merchant app to show paid" 30 '.status == "paid"' curl -sS "$MERCHANT_APP/api/orders/$oid")"
jq -c '{id, status, paid_at}' <<<"$paid"
curl -sS "$MERCHANT_APP/api/webhooks" | jq -c '.[] | select(.type == "invoice.paid")'

say "6. Proof of payment (regtest only)"
preimage="$(jq -r .preimage <<<"$final")"; hash="$(jq -r .payment_hash <<<"$final")"
computed="$(printf '%s' "$preimage" | xxd -r -p | sha256sum | cut -d' ' -f1)"
note "preimage:       $preimage"
note "sha256(preimage) $computed"
[ "$computed" = "$hash" ] && note "matches the payment hash: the payment really settled" || die "preimage does not match the payment hash"

say "7. Failure case: an invoice larger than the channel can carry"
big="$(api POST "$MERCHANT_API" /v1/lightning/invoices "$KA" '{"amount_sat":5000000,"description":"too big"}' "big-$RUN")"
fail="$(api POST "$PAYER_API" /v1/lightning/payments "$KB" "{\"bolt11\":\"$(jq -r .bolt11 <<<"$big")\"}" "pay-$RUN-big")"
jq -c '{id, status, failure_reason}' <<<"$fail"
[ "$(jq -r .status <<<"$fail")" = failed ] && note "definite failure: nothing was sent, the ledger is unchanged" || die "expected a failed payment, got: $fail"

say "8. Idempotent retry: repeating a request returns the stored payment, it does not pay twice"
again="$(api POST "$PAYER_API" /v1/lightning/payments "$KB" "{\"bolt11\":\"$bolt11\"}" "pay-$RUN-ok")"
[ "$(jq -r .id <<<"$again")" = "$pid" ] && note "same payment returned: $pid" || die "retry created a second payment"

say "Done: Demo 1 passed"
