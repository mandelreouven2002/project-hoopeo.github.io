#!/bin/bash
# Removes DEP CLI, DEP Engine and Hoopeo Studio from this Mac.
# Your databases in ~/Library/Application Support/Project Hoopeo are kept; delete that folder to remove them.
set -u
if [ "$(id -u)" != 0 ]; then exec sudo "$0" "$@"; fi
USER_NAME="$(stat -f%Su /dev/console)"
if [ -n "$USER_NAME" ] && [ "$USER_NAME" != root ]; then
  launchctl bootout "gui/$(id -u "$USER_NAME")/click.reuven.hoopeo.dep-engine" 2>/dev/null || true
fi
rm -f /Library/LaunchAgents/click.reuven.hoopeo.dep-engine.plist
rm -f /usr/local/bin/dep-cli /usr/local/bin/dep-engine
rm -rf /usr/local/libexec/project-hoopeo /usr/local/share/project-hoopeo
rm -rf "/Applications/DEP CLI.app" "/Applications/Hoopeo Studio.app"
for p in click.reuven.hoopeo.dep-cli click.reuven.hoopeo.dep-engine click.reuven.hoopeo.dep-engine-service click.reuven.hoopeo.studio; do
  pkgutil --forget "$p" >/dev/null 2>&1 || true
done
echo "Project Hoopeo was removed. Your data in ~/Library/Application Support/Project Hoopeo was kept."
