#!/usr/bin/env bash
# Restart recovery: the merchant node is killed after issuing an invoice. After
# it restarts, the invoice is still known, the channel reconnects, and a payment
# made after the restart is recorded exactly once.
#
# RESTART_MERCHANT_CMD must kill and restart the merchant Safrod process. The
# default suits docker compose; scripts/dev-native.sh sets it for native runs.
source "$(dirname "${BASH_SOURCE[0]}")/lib.sh"
load_keys
RUN="$(date +%s)"
RESTART_MERCHANT_CMD="${RESTART_MERCHANT_CMD:-docker compose kill safrod && docker compose start safrod}"

say "1. Create an invoice, then kill the merchant node hard"
inv="$(api POST "$MERCHANT_API" /v1/lightning/invoices "$KA" '{"amount_sat":1500,"description":"recovery"}' "rec-$RUN")"
iid="$(jq -r .id <<<"$inv")"; bolt11="$(jq -r .bolt11 <<<"$inv")"
note "invoice $iid is open"
eval "$RESTART_MERCHANT_CMD" >/dev/null 2>&1
wait_for "merchant node to come back" 120 ready "$MERCHANT_API"
note "merchant node restarted"

say "2. The invoice survived the restart"
api GET "$MERCHANT_API" "/v1/lightning/invoices/$iid" "$KA" | jq -c '{id, status}'

say "3. Payment after the restart"
wait_for "channel to reconnect" 60 bash -c "curl -s $MERCHANT_API/v1/node -H 'authorization: Bearer $KA' | jq -e '[.channels[] | select(.is_usable)] | length > 0'"
pay="$(api POST "$PAYER_API" /v1/lightning/payments "$KB" "{\"bolt11\":\"$bolt11\"}" "rec-pay-$RUN")"
pid="$(jq -r .id <<<"$pay")"
poll_json "payment to settle" 30 '.status == "succeeded"' api GET "$PAYER_API" "/v1/lightning/payments/$pid" "$KB" | jq -c '{id, status}'
poll_json "invoice to be paid" 30 '.status == "paid"' api GET "$MERCHANT_API" "/v1/lightning/invoices/$iid" "$KA" | jq -c '{id, status, amount_received_msat}'

say "4. Exactly one paid event was recorded"
count="$(api GET "$MERCHANT_API" "/v1/events?type=invoice.paid&limit=100" "$KA" | jq --arg id "$iid" '[.data[] | select(.resource_id == $id)] | length')"
[ "$count" = 1 ] && note "1 invoice.paid event for $iid" || die "expected 1 paid event, found $count"

say "Done: restart recovery passed"
