#!/usr/bin/env bash
# NovaShell GUI watchdog.
#
# systemd's Restart=on-failure does not cover the common WSLg failure: when the
# display server dies, the GTK/WebKit process does not exit, it wedges. The
# service still looks "active" while there is no window on screen, so nothing
# brings the shell back and the desktop just looks empty.
#
# This checks the two things that actually matter - the process and a real
# NovaShell window - and restarts the service when either is missing.
set -uo pipefail

SERVICE="${NOVA_SERVICE:-novashell}"
WAIT_SECONDS="${NOVA_WINDOW_WAIT:-8}"

log() { printf '%s nova-watchdog: %s\n' "$(date -Is)" "$*"; }

process_alive() {
    pgrep -f "novashell --windowed" >/dev/null 2>&1
}

# A window only counts if it can be inspected and has a non-zero size.
window_ok() {
    local wid info
    wid=$(DISPLAY="${DISPLAY:-:0}" xdotool search --name '^NovaShell$' 2>/dev/null | head -1)
    [ -n "$wid" ] || return 1
    info=$(DISPLAY="${DISPLAY:-:0}" xwininfo -id "$wid" 2>/dev/null | tr -d ' ')
    echo "$info" | grep -q 'Width:[1-9]' || return 1
    echo "$info" | grep -q 'Height:[1-9]' || return 1
    return 0
}

if ! systemctl is-active --quiet "$SERVICE"; then
    log "service $SERVICE is not active; starting it"
    systemctl start "$SERVICE"
    exit 0
fi

if process_alive && window_ok; then
    exit 0
fi

# A cold start can take a while to map its window, so do not act on one blip.
for _ in $(seq 1 "$WAIT_SECONDS"); do
    sleep 1
    if process_alive && window_ok; then
        exit 0
    fi
done

if process_alive; then
    log "process alive but no usable window (wedged display); restarting $SERVICE"
else
    log "process gone; restarting $SERVICE"
fi
systemctl restart "$SERVICE"
