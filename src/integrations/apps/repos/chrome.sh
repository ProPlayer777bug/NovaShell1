#!/bin/sh
# Google Chrome — official apt repository.
set -e
arch=$(dpkg --print-architecture)
keydir=/etc/apt/keyrings
install -d -m 0755 "$keydir" /usr/share/keyrings
curl -fsSL https://dl.google.com/linux/linux_signing_key.pub -o "$keydir/google-chrome.gpg"
printf 'deb [arch=%s signed-by=%s/google-chrome.gpg] http://dl.google.com/linux/chrome/deb/ stable main\n' "$arch" "$keydir" > /etc/apt/sources.list.d/google-chrome.list
apt-get update -y