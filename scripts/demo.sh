#!/usr/bin/env bash
# Runs the three demos in order. Usage: scripts/demo.sh [lightning|onchain|sandbox|recovery|all]
cd "$(dirname "${BASH_SOURCE[0]}")"
case "${1:-all}" in
  lightning) ./demo-lightning.sh ;;
  onchain)   ./demo-onchain.sh ;;
  sandbox)   ./demo-sandbox.sh ;;
  recovery)  ./demo-recovery.sh ;;
  all)       ./demo-lightning.sh && ./demo-onchain.sh && ./demo-sandbox.sh && ./demo-recovery.sh ;;
  *) echo "usage: $0 [lightning|onchain|sandbox|recovery|all]" >&2; exit 2 ;;
esac
