#!/usr/bin/env bash
# Runs the whole regtest environment without Docker: bitcoind, two Safrod
# nodes (merchant and payer) and the example merchant app.
#
#   scripts/dev-native.sh start|stop|status|reset|restart-merchant
#
# Requirements: bitcoind and bitcoin-cli (BITCOIN_BIN=/dir or on PATH), node,
# psql, a reachable PostgreSQL whose user may create databases
# (SAFROD_DEV_PG_URL, e.g. postgres://safrod@127.0.0.1:5432), and a built
# daemon (cargo build -p safrod). Never point this at a production database.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DEMO_DIR="${DEMO_DIR:-$ROOT/.demo}"
N="$DEMO_DIR/native"
BIN="${BITCOIN_BIN:-$(dirname "$(command -v bitcoind || echo /nonexistent/bitcoind)")}"
PG="${SAFROD_DEV_PG_URL:?set SAFROD_DEV_PG_URL, e.g. postgres://safrod@127.0.0.1:5432}"
ADMIN="${SAFROD_ADMIN_TOKEN:-dev-admin-token-0123456789abcdef}"
RPC_USER=safrod RPC_PASS=safrodpass RPC_PORT="${RPC_PORT:-18443}"
SAFROD_BIN="${SAFROD_BIN:-$ROOT/target/debug/safrod}"
BCLI="$BIN/bitcoin-cli -regtest -datadir=$N/bitcoin -rpcuser=$RPC_USER -rpcpassword=$RPC_PASS -rpcport=$RPC_PORT"

[ -x "$BIN/bitcoind" ] || { echo "bitcoind not found; set BITCOIN_BIN" >&2; exit 1; }

start_node() { # <name> <api-port> <p2p-port> <alias>
  local name="$1" api="$2" p2p="$3" alias="$4"
  mkdir -p "$N/$name"
  SAFROD_DATABASE_URL="$PG/safrod_$name" SAFROD_BIND="127.0.0.1:$api" \
  SAFROD_LDK_LISTEN="127.0.0.1:$p2p" SAFROD_LDK_STORAGE_DIR="$N/$name" SAFROD_LDK_ALIAS="$alias" \
  SAFROD_BITCOIN_RPC_URL="http://127.0.0.1:$RPC_PORT" SAFROD_BITCOIN_RPC_USER=$RPC_USER \
  SAFROD_BITCOIN_RPC_PASSWORD=$RPC_PASS SAFROD_ADMIN_TOKEN="$ADMIN" SAFROD_ENABLE_DEV_ENDPOINTS=true \
  SAFROD_WEBHOOK_ALLOW_PRIVATE=true RUST_LOG="${RUST_LOG:-info}" \
    nohup "$SAFROD_BIN" >> "$N/$name.log" 2>&1 &
  echo $! > "$N/$name.pid"
}

stop_pid() { [ -f "$N/$1.pid" ] && kill "$(cat "$N/$1.pid")" 2>/dev/null || true; rm -f "$N/$1.pid"; }

case "${1:-}" in
  start)
    mkdir -p "$N/bitcoin"
    [ -x "$SAFROD_BIN" ] || (cd "$ROOT" && cargo build -p safrod)
    for db in merchant payer; do
      psql "$PG/postgres" -qtAc "select 1 from pg_database where datname='safrod_$db'" | grep -q 1 \
        || psql "$PG/postgres" -qc "create database safrod_$db"
    done
    if ! $BCLI getblockchaininfo >/dev/null 2>&1; then
      "$BIN/bitcoind" -regtest -daemon -datadir="$N/bitcoin" -rpcuser=$RPC_USER -rpcpassword=$RPC_PASS \
        -rpcport=$RPC_PORT -port=18444 -fallbackfee=0.0002 -server=1 -listen=0 >/dev/null
      until $BCLI getblockchaininfo >/dev/null 2>&1; do sleep 1; done
    fi
    start_node merchant 8080 9735 merchant
    start_node payer 8081 9736 payer
    MERCHANT_CONFIG="$DEMO_DIR/merchant.json" nohup node "$ROOT/examples/merchant/server.js" >> "$N/app.log" 2>&1 &
    echo $! > "$N/app.pid"
    cat > "$N/env.sh" <<ENV
BITCOIN_CLI="$BCLI"
MERCHANT_P2P=127.0.0.1:9735
MERCHANT_APP_API_URL=http://127.0.0.1:8080
MERCHANT_WEBHOOK_URL=http://127.0.0.1:4000/webhook
RESTART_MERCHANT_CMD="BITCOIN_BIN=$BIN SAFROD_DEV_PG_URL=$PG SAFROD_ADMIN_TOKEN=$ADMIN $ROOT/scripts/dev-native.sh restart-merchant"
ENV
    echo "started. logs: $N/*.log. next: scripts/setup.sh"
    ;;
  restart-merchant)
    # kill -9 on purpose: recovery must not depend on a clean shutdown.
    [ -f "$N/merchant.pid" ] && kill -9 "$(cat "$N/merchant.pid")" 2>/dev/null || true
    sleep 1
    start_node merchant 8080 9735 merchant
    ;;
  stop)
    for p in app merchant payer; do stop_pid "$p"; done
    $BCLI stop >/dev/null 2>&1 || true
    ;;
  status)
    for p in app merchant payer; do
      if [ -f "$N/$p.pid" ] && kill -0 "$(cat "$N/$p.pid")" 2>/dev/null; then echo "$p: running"; else echo "$p: stopped"; fi
    done
    $BCLI getblockchaininfo 2>/dev/null | jq -c '{chain, blocks}' || echo "bitcoind: stopped"
    ;;
  reset)
    "$0" stop || true
    sleep 2
    for db in merchant payer; do psql "$PG/postgres" -qc "drop database if exists safrod_$db" || true; done
    rm -rf "$N" "$DEMO_DIR/keys.env" "$DEMO_DIR/merchant.json"
    echo "reset: databases, regtest chain, node data and keys removed"
    ;;
  *) echo "usage: $0 start|stop|status|reset|restart-merchant" >&2; exit 2 ;;
esac
