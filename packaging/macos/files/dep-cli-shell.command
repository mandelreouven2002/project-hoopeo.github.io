#!/bin/bash
# Opened in Terminal by "DEP CLI.app". Connects to DEP Engine on this Mac if it runs in the
# background (the optional service), or else opens the local database directly.
SUPPORT="$HOME/Library/Application Support/Project Hoopeo"
printf '\033]0;DEP CLI\007'
if /usr/bin/nc -z 127.0.0.1 5433 >/dev/null 2>&1 && [ -r "$SUPPORT/admin-password" ]; then
  export DEP_PASSWORD="$(cat "$SUPPORT/admin-password")"
  exec /usr/local/bin/dep-cli -h 127.0.0.1 -p 5433 -U admin -d operational
else
  exec /usr/local/bin/dep-cli --data-dir "$SUPPORT/data"
fi
