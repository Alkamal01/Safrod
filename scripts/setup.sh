#!/usr/bin/env bash
# Prepares a fresh regtest environment: projects and keys, a funded payer
# node, an open channel from the payer to the merchant node, and a webhook
# endpoint pointing at the example merchant app. Safe to run repeatedly.
source "$(dirname "${BASH_SOURCE[0]}")/lib.sh"

mkdir -p "$DEMO_DIR"
say "Waiting for services"
wait_for "merchant node" 120 ready "$MERCHANT_API"
wait_for "payer node" 120 ready "$PAYER_API"
note "both Safrod nodes are ready (network: regtest)"

say "Projects and API keys"
valid_key() { [ "$(curl -s -o /dev/null -w '%{http_code}' "$1/v1/project" -H "authorization: Bearer $2")" = "200" ]; }
KA="" KB=""
[ -f "$KEYS_FILE" ] && source "$KEYS_FILE"
if [ -z "$KA" ] || ! valid_key "$MERCHANT_API" "$KA"; then
  KA="$(admin POST "$MERCHANT_API" /v1/projects '{"name":"merchant"}' | jq -r .api_key.secret)"
  [ "$KA" != null ] || die "could not create the merchant project (is SAFROD_ADMIN_TOKEN correct?)"
  rm -f "$DEMO_DIR/merchant.json"
fi
if [ -z "$KB" ] || ! valid_key "$PAYER_API" "$KB"; then
  KB="$(admin POST "$PAYER_API" /v1/projects '{"name":"payer"}' | jq -r .api_key.secret)"
  [ "$KB" != null ] || die "could not create the payer project"
fi
printf 'KA=%s\nKB=%s\n' "$KA" "$KB" > "$KEYS_FILE"
chmod 600 "$KEYS_FILE"
note "keys written to $KEYS_FILE (never commit this file)"

say "Bitcoin Core test wallet"
bcli createwallet "$WALLET" >/dev/null 2>&1 || bcli loadwallet "$WALLET" >/dev/null 2>&1 || true
if [ "$(wcli getbalance | cut -d. -f1)" -lt 5 ]; then
  note "mining 101 blocks so coinbase rewards mature"
  mine 101
fi
note "test wallet balance: $(wcli getbalance) BTC (regtest)"

say "Funding the payer node on-chain"
spendable() { api GET "$PAYER_API" /v1/node "$KB" | jq -r .balances.onchain_spendable_sat; }
if [ "$(spendable)" -lt 1500000 ]; then
  payer_addr="$(api POST "$PAYER_API" /v1/bitcoin/addresses "$KB" '{"label":"funding"}' | jq -r .address)"
  txid="$(wcli sendtoaddress "$payer_addr" 1)"
  note "sent 1 BTC to $payer_addr ($txid)"
  mine 3
  wait_for "payer funds" 60 bash -c "[ \"\$(curl -s $PAYER_API/v1/node -H 'authorization: Bearer $KB' | jq -r .balances.onchain_spendable_sat)\" -ge 1500000 ]"
fi
note "payer spendable: $(spendable) sat"

say "Channel from payer to merchant"
merchant_id="$(api GET "$MERCHANT_API" /v1/node "$KA" | jq -r .info.node_id)"
has_channel() { api GET "$PAYER_API" /v1/node "$KB" | jq -e --arg id "$merchant_id" '[.channels[] | select(.peer_node_id == $id and .is_usable)] | length > 0' >/dev/null; }
if ! has_channel; then
  admin POST "$PAYER_API" /v1/admin/channels \
    "{\"peer_node_id\":\"$merchant_id\",\"peer_address\":\"$MERCHANT_P2P\",\"capacity_sat\":1000000,\"push_msat\":0}" | jq -c .
  sleep 3
  mine 6
  wait_for "channel to become usable" 90 has_channel
fi
api GET "$PAYER_API" /v1/node "$KB" | jq -c '.channels[] | {capacity_sat, outbound_capacity_msat, is_usable}'

say "Webhook endpoint for the merchant app"
has_endpoint() { api GET "$MERCHANT_API" /v1/webhook-endpoints "$KA" | jq -e --arg u "$MERCHANT_WEBHOOK_URL" '[.data[] | select(.url == $u and .enabled)] | length > 0' >/dev/null; }
if [ ! -f "$DEMO_DIR/merchant.json" ] || ! has_endpoint; then
  api GET "$MERCHANT_API" /v1/webhook-endpoints "$KA" | jq -r '.data[].id' | while read -r old; do
    api DELETE "$MERCHANT_API" "/v1/webhook-endpoints/$old" "$KA" >/dev/null
  done
  secret="$(api POST "$MERCHANT_API" /v1/webhook-endpoints "$KA" "{\"url\":\"$MERCHANT_WEBHOOK_URL\"}" | jq -r .secret)"
  [ "$secret" != null ] || die "could not register the webhook endpoint (is SAFROD_WEBHOOK_ALLOW_PRIVATE=true for this local app?)"
  jq -n --arg u "$MERCHANT_APP_API_URL" --arg k "$KA" --arg s "$secret" \
    '{api_url: $u, api_key: $k, webhook_secret: $s}' > "$DEMO_DIR/merchant.json"
  chmod 600 "$DEMO_DIR/merchant.json"
fi
note "merchant app config: $DEMO_DIR/merchant.json"

say "Ready"
note "next: scripts/demo.sh   (or demo-lightning.sh, demo-onchain.sh, demo-sandbox.sh)"
