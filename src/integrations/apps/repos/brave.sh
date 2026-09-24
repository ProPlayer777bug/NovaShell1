#!/bin/sh
# Brave Browser — official apt repository (https://brave.com/linux).
set -e
arch=$(dpkg --print-architecture)
keydir=/etc/apt/keyrings
install -d -m 0755 "$keydir"
curl -fsSL https://brave-browser-apt-release.s3.brave.com/brave-browser-archive-keyring.gpg -o "$keydir/brave-browser-archive-keyring.gpg"
printf 'deb [signed-by=%s/brave-browser-archive-keyring.gpg arch=%s] https://brave-browser-apt-release.s3.brave.com/ stable main\n' "$keydir" "$arch" > /etc/apt/sources.list.d/brave-browser-release.list
apt-get update -y