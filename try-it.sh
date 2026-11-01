#!/bin/bash
# Tries Project Hoopeo from the source, without installing anything:
#
#   ./try-it.sh                 DEP CLI on the demo clinic, in memory
#   ./try-it.sh tour            the guided tour, every statement with its result and DEP notes
#   ./try-it.sh server          DEP Engine on 127.0.0.1:5433 (and HTTPS on 8433) with the demo clinic (admin password: secret)
#   ./try-it.sh studio          Hoopeo Studio
#   ./try-it.sh cli ARGS...     DEP CLI on the demo clinic, with more arguments (for example -f FILE)
#
# The first run compiles everything, which takes a few minutes.
set -euo pipefail
cd "$(dirname "$0")"
ROOT="$PWD"
case "${1:-cli}" in
  tour)   cargo build --release -q -p dep-cli && (cd public && "$ROOT/target/release/dep-cli" --memory --init examples/clinic.sql -f examples/tour/all.sql) ;;
  server) cargo run --release -q -p dep-engine -- --listen 127.0.0.1:5433 --init examples/clinic.sql --admin-password secret --tls-self-signed --http-listen 127.0.0.1:8433 ;;
  studio) cargo run --release -q -p hoopeo-studio ;;
  cli)    shift || true; cargo run --release -q -p dep-cli -- --memory --init examples/clinic.sql "$@" ;;
  *)      sed -n '2,10p' "$0"; exit 2 ;;
esac
