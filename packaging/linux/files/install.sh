#!/bin/sh
# Installs DEP Engine from this archive as a systemd service:
#   sudo ./install.sh            install, create the dep-engine user, start the service
#   sudo ./install.sh --no-start install without starting
#   sudo ./install.sh --uninstall
# The database is kept in /var/lib/dep-engine; remove it yourself to delete the data.
set -eu
HERE="$(cd "$(dirname "$0")" && pwd)"
[ "$(id -u)" = 0 ] || { echo "Run as root: sudo $0 $*" >&2; exit 1; }
if [ "${1:-}" = "--uninstall" ]; then
  systemctl disable --now dep-engine 2>/dev/null || true
  rm -f /etc/systemd/system/dep-engine.service /usr/local/bin/dep-engine /usr/local/bin/dep-cli
  systemctl daemon-reload 2>/dev/null || true
  echo "DEP Engine was removed. /etc/dep-engine and /var/lib/dep-engine were kept."
  exit 0
fi
install -m 0755 "$HERE/bin/dep-engine" /usr/local/bin/dep-engine
[ -f "$HERE/bin/dep-cli" ] && install -m 0755 "$HERE/bin/dep-cli" /usr/local/bin/dep-cli
id dep-engine >/dev/null 2>&1 || useradd --system --home-dir /var/lib/dep-engine --shell /usr/sbin/nologin --user-group dep-engine
install -d -m 0750 -o root -g dep-engine /etc/dep-engine
[ -f /etc/dep-engine/dep-engine.env ] || install -m 0640 -o root -g dep-engine "$HERE/etc/dep-engine.env" /etc/dep-engine/dep-engine.env
if [ ! -f /etc/dep-engine/admin-password ]; then
  umask 077
  head -c 12 /dev/urandom | od -An -tx1 | tr -d ' \n' > /etc/dep-engine/admin-password
  chown root:dep-engine /etc/dep-engine/admin-password
  chmod 0640 /etc/dep-engine/admin-password
fi
install -d -m 0700 -o dep-engine -g dep-engine /var/lib/dep-engine
sed 's#/usr/bin/dep-engine#/usr/local/bin/dep-engine#' "$HERE/etc/dep-engine.service" > /etc/systemd/system/dep-engine.service
systemctl daemon-reload
if [ "${1:-}" != "--no-start" ]; then systemctl enable --now dep-engine; fi
echo "DEP Engine is installed. Configuration: /etc/dep-engine/dep-engine.env"
echo "The admin password is in /etc/dep-engine/admin-password (readable by root)."
echo "Connect: DEP_PASSWORD=\$(sudo cat /etc/dep-engine/admin-password) dep-cli -h 127.0.0.1 -p 5433 -U admin -d operational"
