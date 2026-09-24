#!/bin/bash
# Install PS2 BIOS images into PCSX2's data directory so the emulator stops
# asking for a BIOS on every start.
#
# PCSX2 (2.x) reads BIOS from <data dir>/bios — by default
# ~/.config/PCSX2/bios — and refuses to boot a game without one. This script
# unpacks a zip of BIOS images there and also provides the canonical file
# names PCSX2 looks for.
#
# Usage:
#   scripts/install-ps2-bios.sh [zip-or-directory] [user]
#
# Defaults to the archive shipped alongside the project, then to the user's
# Windows "mygame forge" folder, and runs for the given user (default: the
# current user, or `aara` when run as root).
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"

SOURCE="${1:-}"
TARGET_USER="${2:-${SUDO_USER:-${USER:-aara}}}"

if [ -z "$SOURCE" ]; then
  for candidate in \
    "$PROJECT_DIR/ps2-bios-all-bios.zip" \
    "/mnt/d/Games/mygame forge/ps2-bios-all-bios.zip" \
    "/usr/share/novashell/ps2-bios-all-bios.zip"; do
    if [ -f "$candidate" ]; then
      SOURCE="$candidate"
      break
    fi
  done
fi

if [ -z "$SOURCE" ] || [ ! -e "$SOURCE" ]; then
  echo "install-ps2-bios: no BIOS archive found." >&2
  echo "Pass a path: scripts/install-ps2-bios.sh /path/to/bios.zip [user]" >&2
  exit 1
fi

TARGET_HOME="$(getent passwd "$TARGET_USER" | cut -d: -f6)"
[ -n "$TARGET_HOME" ] || TARGET_HOME="/home/$TARGET_USER"
BIOS_DIR="$TARGET_HOME/.config/PCSX2/bios"

echo "install-ps2-bios: source   = $SOURCE"
echo "install-ps2-bios: installing into $BIOS_DIR"

install -d -m 0755 "$BIOS_DIR"

python3 - "$SOURCE" "$BIOS_DIR" <<'PY'
import os, shutil, sys, zipfile

source, bios_dir = sys.argv[1], sys.argv[2]
names = []

if zipfile.is_zipfile(source):
    with zipfile.ZipFile(source) as z:
        for entry in z.namelist():
            if entry.endswith('/'):
                continue
            base = os.path.basename(entry)
            if not base:
                continue
            dest = os.path.join(bios_dir, base)
            with z.open(entry) as src, open(dest, 'wb') as out:
                shutil.copyfileobj(src, out)
            os.chmod(dest, 0o644)
            names.append(base)
else:
    for base in os.listdir(source):
        src = os.path.join(source, base)
        if not os.path.isfile(src):
            continue
        dest = os.path.join(bios_dir, base)
        shutil.copy2(src, dest)
        os.chmod(dest, 0o644)
        names.append(base)

print(f"install-ps2-bios: unpacked {len(names)} file(s)")

# PCSX2 looks for specific names; provide them from the images we unpacked so
# it finds a usable BIOS whichever name it expects.
def pick(*patterns):
    for n in names:
        low = n.lower()
        if any(p in low for p in patterns):
            return os.path.join(bios_dir, n)
    return None

def link_as(target_name, *patterns):
    target = os.path.join(bios_dir, target_name)
    if os.path.exists(target):
        return
    src = pick(*patterns)
    if src:
        shutil.copy2(src, target)
        os.chmod(target, 0o644)
        print(f"install-ps2-bios: {target_name} <- {os.path.basename(src)}")

link_as('scph55000.bin', 'scph55000', '30004r', 'rom1.bin', 'scph39001')
link_as('scph55001.bin', 'scph55001', '70004', 'scph10000')
link_as('scph55001.EROM', 'erom', 'rom2')
link_as('scph55001.NVM', 'nvm')
link_as('scph55001.MEC', 'mec')
link_as('rom1.bin', 'rom1')
PY

# A second copy in the legacy inis/bios location costs nothing and covers
# PCSX2 builds that still read it from there.
if [ -d "$TARGET_HOME/.config/PCSX2/inis" ]; then
  cp -f "$BIOS_DIR"/* "$TARGET_HOME/.config/PCSX2/inis/bios/" 2>/dev/null || true
fi

if [ "$(id -u)" -eq 0 ]; then
  chown -R "$TARGET_USER":"$TARGET_USER" "$TARGET_HOME/.config/PCSX2" 2>/dev/null || true
fi

echo "install-ps2-bios: done. BIOS now in $BIOS_DIR"
ls -1 "$BIOS_DIR" | sed 's/^/  /'
