#!/bin/bash
# Install NovaShell as a systemd service so it starts on boot and stays running
# (a manually launched shell is killed when the WSL session that started it ends).
#
# Also installs a watchdog timer. The service's Restart=on-failure cannot recover
# from a WSLg display death: the GTK process wedges instead of exiting, so the
# unit still looks active while the desktop has no window. The watchdog checks
# for a real window and restarts the service when one is missing.
set -e
DEST_USER="${1:-aara}"

if [ "$(id -u)" -ne 0 ]; then
  echo "install-service.sh must run as root (try: sudo $0 $DEST_USER)" >&2
  exit 1
fi
if ! getent passwd "$DEST_USER" >/dev/null; then
  echo "no such user: $DEST_USER" >&2
  exit 1
fi
DEST_UID="$(id -u "$DEST_USER")"

install -d -m 0755 /etc/systemd/system /etc/tmpfiles.d /usr/local/bin
sed "s/^User=aara$/User=$DEST_USER/; s#XDG_RUNTIME_DIR=/run/user/1000#XDG_RUNTIME_DIR=/run/user/$DEST_UID#; s#chown aara:aara /run/user/1000#chown $DEST_USER:$DEST_USER /run/user/$DEST_UID#; s#^Environment=HOME=.*#Environment=HOME=$(getent passwd "$DEST_USER" | cut -d: -f6)#; s#^WorkingDirectory=.*#WorkingDirectory=$(getent passwd "$DEST_USER" | cut -d: -f6)#" \
  "$(dirname "$0")/../data/novashell.service" > /etc/systemd/system/novashell.service
chmod 644 /etc/systemd/system/novashell.service

sed "s#/run/user/1000#/run/user/$DEST_UID#; s/ aara aara / $DEST_USER $DEST_USER /" \
  "$(dirname "$0")/../data/tmpfiles-novashell.conf" > /etc/tmpfiles.d/novashell.conf
chmod 644 /etc/tmpfiles.d/novashell.conf

# GUI watchdog: catches a dead window that Restart=on-failure cannot.
install -m 0755 "$(dirname "$0")/nova-watchdog.sh" /usr/local/bin/nova-watchdog
install -m 0644 "$(dirname "$0")/../data/novashell-watchdog.service" /etc/systemd/system/novashell-watchdog.service
install -m 0644 "$(dirname "$0")/../data/novashell-watchdog.timer" /etc/systemd/system/novashell-watchdog.timer

systemctl daemon-reload
systemd-tmpfiles --create /etc/tmpfiles.d/novashell.conf || true
systemctl enable novashell.service >/dev/null 2>&1 || true
systemctl enable --now novashell-watchdog.timer >/dev/null 2>&1 || true
systemctl restart novashell.service
sleep 4
systemctl is-active novashell.service
systemctl is-active novashell-watchdog.timer >/dev/null 2>&1 \
  && echo "watchdog timer active" \
  || echo "watchdog timer NOT active (the shell will not self-heal)"
