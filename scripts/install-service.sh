#!/bin/bash
# Install NovaShell as a systemd service so it starts on boot and stays running
# (a manually launched shell is killed when the WSL session that started it ends).
set -e
DEST_USER="${1:-aara}"

install -d -m 0755 /etc/systemd/system /etc/tmpfiles.d
sed "s/^User=aara$/User=$DEST_USER/; s#XDG_RUNTIME_DIR=/run/user/1000#XDG_RUNTIME_DIR=/run/user/$(id -u "$DEST_USER")#; s#chown aara:aara /run/user/1000#chown $DEST_USER:$DEST_USER /run/user/$(id -u "$DEST_USER")#" \
  "$(dirname "$0")/../data/novashell.service" > /etc/systemd/system/novashell.service
chmod 644 /etc/systemd/system/novashell.service

sed "s#/run/user/1000#/run/user/$(id -u "$DEST_USER")#; s/ aara aara / $DEST_USER $DEST_USER /" \
  "$(dirname "$0")/../data/tmpfiles-novashell.conf" > /etc/tmpfiles.d/novashell.conf
chmod 644 /etc/tmpfiles.d/novashell.conf

systemctl daemon-reload
systemd-tmpfiles --create /etc/tmpfiles.d/novashell.conf || true
systemctl enable novashell.service >/dev/null 2>&1 || true
systemctl restart novashell.service
sleep 4
systemctl is-active novashell.service
