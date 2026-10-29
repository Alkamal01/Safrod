#!/usr/bin/env bash
# Demo 2, on-chain: merchant address -> test wallet pays -> blocks are mined
# -> confirmations update -> events and webhooks.
source "$(dirname "${BASH_SOURCE[0]}")/lib.sh"
load_keys

say "1. Merchant app creates an on-chain order (address from the node wallet)"
order="$(curl -sS -X POST "$MERCHANT_APP/api/orders" -d '{"kind":"onchain"}')"
oid="$(jq -r .id <<<"$order")"; addr="$(jq -r .address <<<"$order")"
[ "$addr" != null ] || die "order creation failed: $order"
note "order: $oid"
note "address: $addr (requires 3 confirmations)"

say "2. The Bitcoin Core test wallet pays it"
txid="$(wcli sendtoaddress "$addr" 0.0005)"
note "txid: $txid"

say "3. Deposit detected in the mempool"
tx="$(poll_json "deposit to be detected" 30 'length > 0 and .[0].txid != null' bash -c "curl -s '$MERCHANT_API/v1/bitcoin/transactions' -H 'authorization: Bearer $KA' | jq '[.data[] | select(.txid == \"$txid\")]'")"
jq -c '.[0] | {txid, amount_sat, status, confirmations}' <<<"$tx"

say "4. Mining blocks; confirmations update"
for n in 1 2 3; do
  mine 1
  state="$(poll_json "confirmation $n" 30 ".[0].confirmations >= $n" bash -c "curl -s '$MERCHANT_API/v1/bitcoin/transactions' -H 'authorization: Bearer $KA' | jq '[.data[] | select(.txid == \"$txid\")]'")"
  jq -c '.[0] | {status, confirmations}' <<<"$state"
done

say "5. Merchant app shows the order paid (via signed webhook)"
paid="$(poll_json "merchant app to show paid" 30 '.status == "paid"' curl -sS "$MERCHANT_APP/api/orders/$oid")"
jq -c '{id, status, confirmations, txid}' <<<"$paid"

say "6. Event log"
api GET "$MERCHANT_API" "/v1/events?type=bitcoin.transaction.confirmed" "$KA" | jq -c '.data[] | {seq, type, resource_id}'
curl -sS "$MERCHANT_APP/api/webhooks" | jq -c '.[] | select((.type // "") | startswith("bitcoin."))' | tail -4

say "Done: Demo 2 passed"
