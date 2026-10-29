#!/usr/bin/env bash
# Demo 3, integration sandbox. THE FIAT SIDE IS SIMULATED: no NGN or KES moves.
# The Bitcoin leg is real regtest Lightning, and is released only after the
# simulated destination payout is confirmed.
source "$(dirname "${BASH_SOURCE[0]}")/lib.sh"
load_keys
RUN="$(date +%s)"

printf '\n\033[1;33m!! SIMULATION BOUNDARY !!\033[0m\n'
note "Quotes, balances and payout outcomes below are synthetic (simulated_fiat: true)."
note "Only the BTC settlement invoice is paid over real regtest Lightning."

say "1. Fund a simulated NGN balance for the source provider (payer project)"
api POST "$PAYER_API" /v1/sandbox/balances "$KB" '{"currency":"NGN","minor_units":5000000}' | jq -c .

scenario() { # scenario <name> <expected-payout-status>
  local name="$1"
  say "Scenario: $name"
  quote="$(api POST "$PAYER_API" /v1/sandbox/quotes "$KB" '{"source":{"currency":"NGN","minor_units":500000},"target_currency":"KES"}')"
  qid="$(jq -r .id <<<"$quote")"; msat="$(jq -r .btc_amount_msat <<<"$quote")"
  jq -c '{id, source, target, btc_amount_msat, simulated_fiat}' <<<"$quote"
  # The destination provider (merchant node) issues the invoice for the BTC leg.
  dest="$(api POST "$MERCHANT_API" /v1/lightning/invoices "$KA" "{\"amount_msat\":$msat,\"description\":\"sandbox $name\"}" "sbx-$RUN-$name")"
  dest_id="$(jq -r .id <<<"$dest")"
  payout="$(api POST "$PAYER_API" /v1/sandbox/payouts "$KB" \
    "{\"quote_id\":\"$qid\",\"scenario\":\"$name\",\"settlement_invoice\":\"$(jq -r .bolt11 <<<"$dest")\"}" "payout-$RUN-$name")"
  jq -c '{id, status, scenario, payment_id, simulated_fiat, real_regtest_lightning}' <<<"$payout"
  PAYOUT_ID="$(jq -r .id <<<"$payout")"; DEST_ID="$dest_id"
}
invoice_status() { api GET "$MERCHANT_API" "/v1/lightning/invoices/$DEST_ID" "$KA" | jq -r .status; }

scenario failure
note "payout failed -> simulated funds refunded, Lightning never released"
[ "$(invoice_status)" = open ] || die "Bitcoin was released for a failed payout"
api GET "$PAYER_API" /v1/sandbox/balances "$KB" | jq -c '.balances'

scenario timeout
note "ambiguous outcome -> payout is 'unknown'; a timeout does not prove failure, so no Bitcoin moves"
[ "$(invoice_status)" = open ] || die "Bitcoin was released for an unknown payout"
note "destination later confirms the payout; reconciling"
resolved="$(api POST "$PAYER_API" "/v1/sandbox/payouts/$PAYOUT_ID/reconcile" "$KB" '{"outcome":"success"}')"
jq -c '{id, status, payment_id}' <<<"$resolved"
poll_json "settlement invoice to be paid" 30 '.status == "paid"' api GET "$MERCHANT_API" "/v1/lightning/invoices/$DEST_ID" "$KA" | jq -c '{id, status, amount_received_msat}'

scenario success
poll_json "settlement invoice to be paid" 30 '.status == "paid"' api GET "$MERCHANT_API" "/v1/lightning/invoices/$DEST_ID" "$KA" | jq -c '{id, status, amount_received_msat}'
api GET "$PAYER_API" "/v1/sandbox/payouts/$PAYOUT_ID" "$KB" | jq -c '{status, payment: {status: .payment.status, real_regtest_lightning: .payment.real_regtest_lightning}}'

say "Balances keep real and simulated amounts apart"
api GET "$PAYER_API" /v1/balances "$KB" | jq '{ledger: {sent_msat: .ledger.lightning_sent_msat}, simulated_fiat}'

say "Done: Demo 3 passed (fiat simulated, BTC settlement real on regtest)"
