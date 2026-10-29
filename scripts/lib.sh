#!/usr/bin/env bash
# Shared helpers for the regtest setup and demo scripts. Source, don't run.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DEMO_DIR="${DEMO_DIR:-$ROOT/.demo}"
KEYS_FILE="$DEMO_DIR/keys.env"

# Native runs (scripts/dev-native.sh) write overrides here.
[ -f "$DEMO_DIR/native/env.sh" ] && source "$DEMO_DIR/native/env.sh"

# Defaults match docker-compose.yml.
SAFROD_ADMIN_TOKEN="${SAFROD_ADMIN_TOKEN:-dev-admin-token-0123456789abcdef}"
MERCHANT_API="${MERCHANT_API:-http://127.0.0.1:8080}"
PAYER_API="${PAYER_API:-http://127.0.0.1:8081}"
MERCHANT_APP="${MERCHANT_APP:-http://127.0.0.1:4000}"
# How the payer node reaches the merchant node's Lightning port.
MERCHANT_P2P="${MERCHANT_P2P:-safrod:9735}"
# How the merchant app reaches the Safrod API, and how Safrod reaches the app.
MERCHANT_APP_API_URL="${MERCHANT_APP_API_URL:-http://safrod:8080}"
MERCHANT_WEBHOOK_URL="${MERCHANT_WEBHOOK_URL:-http://merchant:4000/webhook}"
BITCOIN_CLI="${BITCOIN_CLI:-docker compose exec -T bitcoind bitcoin-cli -regtest -rpcuser=safrod -rpcpassword=${BITCOIND_RPC_PASSWORD:-safrodpass}}"
WALLET="${WALLET:-demo}"

say()  { printf '\n\033[1m== %s\033[0m\n' "$*"; }
note() { printf '   %s\n' "$*"; }
die()  { printf 'error: %s\n' "$*" >&2; exit 1; }

need() { command -v "$1" >/dev/null 2>&1 || die "$1 is required"; }

bcli() { $BITCOIN_CLI "$@"; }
wcli() { $BITCOIN_CLI -rpcwallet="$WALLET" "$@"; }

mine() {
  local addr
  addr="$(wcli getnewaddress)"
  bcli generatetoaddress "$1" "$addr" >/dev/null
}

# api <method> <base-url> <path> <key> [json-body] [idempotency-key]
api() {
  local method="$1" base="$2" path="$3" key="$4" body="${5:-}" idem="${6:-}"
  local args=(-sS -X "$method" "$base$path" -H "authorization: Bearer $key")
  [ -n "$body" ] && args+=(-H 'content-type: application/json' -d "$body")
  [ -n "$idem" ] && args+=(-H "Idempotency-Key: $idem")
  curl "${args[@]}"
}

admin() {
  local method="$1" base="$2" path="$3" body="${4:-}"
  local args=(-sS -X "$method" "$base$path" -H "x-admin-token: $SAFROD_ADMIN_TOKEN")
  [ -n "$body" ] && args+=(-H 'content-type: application/json' -d "$body")
  curl "${args[@]}"
}

wait_for() { # wait_for <description> <timeout-secs> <command...>
  local what="$1" timeout="$2"; shift 2
  local i=0
  until "$@" >/dev/null 2>&1; do
    i=$((i + 1))
    [ "$i" -ge "$timeout" ] && die "timed out waiting for $what"
    sleep 1
  done
}

ready() { [ "$(curl -s -o /dev/null -w '%{http_code}' "$1/readyz")" = "200" ]; }

load_keys() {
  [ -f "$KEYS_FILE" ] || die "$KEYS_FILE not found; run scripts/setup.sh first"
  # shellcheck disable=SC1090
  source "$KEYS_FILE"
}

# poll_json <description> <timeout> <jq-condition> <command...>: re-runs the
# command until its JSON output satisfies the jq condition; prints the result.
poll_json() {
  local what="$1" timeout="$2" cond="$3"; shift 3
  local i=0 out
  while :; do
    out="$("$@" 2>/dev/null || true)"
    if jq -e "$cond" >/dev/null 2>&1 <<<"$out"; then
      printf '%s' "$out"
      return 0
    fi
    i=$((i + 1))
    [ "$i" -ge "$timeout" ] && die "timed out waiting for $what (last: $out)"
    sleep 1
  done
}

need curl
need jq
