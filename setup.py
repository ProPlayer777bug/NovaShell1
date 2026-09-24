#!/usr/bin/env python3
"""NovaShell setup: build and install on Ubuntu/Debian.

Usage:
    python3 setup.py                # system-wide install (needs sudo)
    python3 setup.py --user         # per-user install (no root)
    python3 setup.py --check        # only verify toolchain + system deps
    python3 setup.py --uninstall    # remove a previous install

Safe by design (same guarantees as scripts/install.sh):
    - never modifies GRUB, kernel params, initramfs, /etc/fstab,
      display managers, or GNOME configuration
    - GNOME stays installed and reachable as the desktop fallback
    - only writes to standard app / icon / autostart / systemd locations

Requires: python3 >= 3.8 (stdlib only), cargo + system dev headers.
"""

import argparse
import os
import platform
import shutil
import subprocess
import sys
from pathlib import Path

APP_NAME = "novashell"
BIN_NAME = "novashell"
ICON_SRC = "assets/org.novashell.svg"
DESKTOP_SRC = "data/novashell.desktop"
AUTOSTART_SRC = "data/autostart/novashell.desktop"
SERVICE_SRC = "data/systemd/novashell.service"

BUILD_DEPS = [
    "curl",
    "build-essential",
    "pkg-config",
    "libgtk-4-dev",
    "libwebkitgtk-6.0-dev",
    "libjavascriptcoregtk-6.0-dev",
    "libudev-dev",
]

# Runtime helpers used opportunistically (UI degrades if absent).
RUNTIME_HINTS = [
    ("wpctl", "volume control (WirePlumber/PipeWire)"),
    ("brightnessctl", "backlight control"),
    ("grim", "Wayland screenshots"),
    ("gnome-screenshot", "X11 screenshots"),
    ("nmcli", "Wi-Fi SSID in status bar"),
    ("bluetoothctl", "Bluetooth device list"),
    ("systemctl", "power actions"),
    ("gnome-session-quit", "log out action"),
]


def log(msg: str) -> None:
    print(f"==> {msg}", flush=True)


def warn(msg: str) -> None:
    print(f"    WARNING: {msg}", flush=True)


def run(cmd, **kw) -> subprocess.CompletedProcess:
    return subprocess.run(cmd, check=False, **kw)


def which(bin: str) -> Path | None:
    return Path(shutil.which(bin)) if shutil.which(bin) else None


def is_deb_based() -> bool:
    return os.path.exists("/etc/debian_version") or os.path.exists("/etc/os-release") and "debian" in (
        (Path("/etc/os-release").read_text() if os.path.exists("/etc/os-release") else "")
    ).lower()


def project_root() -> Path:
    """Repo root = the folder containing this setup.py."""
    return Path(__file__).resolve().parent


def check_rust() -> Path:
    cargo = which("cargo")
    if cargo is None:
        log("Rust toolchain not found. Install it first:")
        print(
            "    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh"
        )
        print("    then:  source ~/.cargo/env   (or log out/in)")
        sys.exit(1)
    ver = subprocess.run(
        ["rustc", "--version"], capture_output=True, text=True
    ).stdout.strip()
    log(f"rustc: {ver}")
    # Scripts/build.sh requires rustc >= 1.85 (webkit6 needs edition 2024).
    m = None
    import re

    m = re.search(r"rustc (\d+)\.(\d+)", ver)
    if m:
        maj, minor = int(m.group(1)), int(m.group(2))
        if maj == 1 and minor >= 85:
            return cargo
    warn("rustc < 1.85 detected; update with:  rustup update stable")
    return cargo


def check_dev_headers() -> None:
    if which("pkg-config") is None:
        print("    pkg-config not found. Run: sudo apt install pkg-config")
        sys.exit(1)
    missing = []
    for pkg in ("gtk4", "javascriptcoregtk-6.0", "webkitgtk-6.0"):
        if run(["pkg-config", "--exists", pkg]).returncode != 0:
            missing.append(pkg)
    if missing:
        print(f"    missing dev packages: {', '.join(missing)}")
        print("    Install them:  sudo apt install " + " ".join(BUILD_DEPS))
        sys.exit(1)
    for pkg in ("gtk4", "javascriptcoregtk-6.0", "webkitgtk-6.0"):
        v = subprocess.run(
            ["pkg-config", "--modversion", pkg], capture_output=True, text=True
        ).stdout.strip()
        print(f"    found: {pkg} {v}")


def install_system_deps() -> None:
    if not is_deb_based():
        log("Not a Debian/Ubuntu system; skipping apt dependency install.")
        print("    Install equivalents yourself: " + " ".join(BUILD_DEPS))
        return
    if which("apt-get") is None:
        warn("apt-get not found; skipping system dependency install.")
        return
    need = [d for d in BUILD_DEPS if run(["pkg-config", "--exists", d]).returncode != 0]
    if not need and which("build-essential") is not None:
        # build-essential has no pkg-config test; just run full install for safety.
        need = BUILD_DEPS
    log(f"installing system packages: {' '.join(need)}")
    r = run(["sudo", "-n", "true"])
    if r.returncode != 0:
        run(["sudo", "apt-get", "update"])
    else:
        run(["sudo", "-n", "apt-get", "update"])
    r = run(["sudo", "-n", "apt-get", "install", "-y"] + need)
    if r.returncode != 0:
        # fall back to interactive sudo
        run(["sudo", "apt-get", "install", "-y"] + need)


def build() -> None:
    log("building release binary (cargo build --release)")
    r = run(["cargo", "build", "--release"], cwd=str(project_root()))
    if r.returncode != 0:
        print("    build failed. See output above.")
        sys.exit(1)
    bin_path = project_root() / "target" / "release" / BIN_NAME
    if not bin_path.exists():
        print(f"    expected binary not found: {bin_path}")
        sys.exit(1)


def install_files(user: bool) -> None:
    home = Path.home()
    if user:
        bin_dir = Path(os.environ.get("NOVA_BIN_DIR", str(home / ".local" / "bin")))
        apps_dir = home / ".local" / "share" / "applications"
        icons_dir = home / ".local" / "share" / "icons" / "hicolor" / "scalable" / "apps"
        autostart_dir = home / ".config" / "autostart"
        systemd_dir = home / ".config" / "systemd" / "user"
    else:
        if os.geteuid() != 0:
            print("System-wide install needs root. Re-run with sudo, or use --user.")
            sys.exit(1)
        bin_dir = Path("/usr/local/bin")
        apps_dir = Path("/usr/local/share/applications")
        icons_dir = Path("/usr/local/share/icons/hicolor/scalable/apps")
        autostart_dir = Path("/etc/xdg/autostart")
        systemd_dir = Path("/etc/systemd/user")

    for d in (bin_dir, apps_dir, icons_dir, autostart_dir, systemd_dir):
        d.mkdir(parents=True, exist_ok=True)

    root = project_root()
    shutil.copy2(root / "target" / "release" / BIN_NAME, bin_dir / BIN_NAME)
    os.chmod(bin_dir / BIN_NAME, 0o755)
    shutil.copy2(root / ICON_SRC, icons_dir / "org.novashell.svg")
    shutil.copy2(root / DESKTOP_SRC, apps_dir / "novashell.desktop")

    autostart = (autostart_dir / "novashell.desktop").read_text()
    autostart = autostart.replace("@BINARY@", str(bin_dir / BIN_NAME))
    (autostart_dir / "novashell.desktop").write_text(autostart)

    service = (systemd_dir / "novashell.service").read_text()
    service = service.replace("@BINARY@", str(bin_dir / BIN_NAME))
    (systemd_dir / "novashell.service").write_text(service)

    log("installed to:")
    for f in (bin_dir / BIN_NAME, apps_dir / "novashell.desktop",
              icons_dir / "org.novashell.svg", autostart_dir / "novashell.desktop",
              systemd_dir / "novashell.service"):
        print(f"    {f}")

    # Refresh icon cache + systemd user units, best effort.
    if which("gtk-update-icon-cache"):
        run(["gtk-update-icon-cache", str(icons_dir.parent.parent.parent)], check=False)
    if which("systemctl"):
        run(["systemctl", "--user", "daemon-reload"], check=False)


def uninstall(user: bool) -> None:
    home = Path.home()
    if user:
        bin_dir = Path(os.environ.get("NOVA_BIN_DIR", str(home / ".local" / "bin")))
        apps_dir = home / ".local" / "share" / "applications"
        icons_dir = home / ".local" / "share" / "icons" / "hicolor" / "scalable" / "apps"
        autostart_dir = home / ".config" / "autostart"
        systemd_dir = home / ".config" / "systemd" / "user"
    else:
        if os.geteuid() != 0:
            print("System-wide uninstall needs root. Re-run with sudo, or use --user.")
            sys.exit(1)
        bin_dir, apps_dir = Path("/usr/local/bin"), Path("/usr/local/share/applications")
        icons_dir = Path("/usr/local/share/icons/hicolor/scalable/apps")
        autostart_dir, systemd_dir = Path("/etc/xdg/autostart"), Path("/etc/systemd/user")

    for f in (
        bin_dir / BIN_NAME,
        apps_dir / "novashell.desktop",
        icons_dir / "org.novashell.svg",
        autostart_dir / "novashell.desktop",
        systemd_dir / "novashell.service",
    ):
        if f.exists():
            f.unlink()
            print(f"    removed {f}")
    if which("systemctl"):
        run(["systemctl", "--user", "daemon-reload"], check=False)


def check_mode() -> None:
    log("checking environment")
    check_rust()
    check_dev_headers()
    for tool, use in RUNTIME_HINTS:
        print(f"    {'OK ' if which(tool) else '---'} {tool:16s} {use}")
    print("    Environment OK: dependencies are satisfied.")


def main() -> None:
    ap = argparse.ArgumentParser(description="Build and install NovaShell")
    ap.add_argument("--user", action="store_true", help="per-user install")
    ap.add_argument("--check", action="store_true", help="verify toolchain + deps only")
    ap.add_argument("--uninstall", action="store_true", help="remove NovaShell")
    args = ap.parse_args()

    if platform.system() != "Linux":
        print("NovaShell targets Ubuntu/Debian Linux. This looks like:", platform.system())
        sys.exit(1)

    if args.check:
        check_mode()
        return
    if args.uninstall:
        uninstall(user=args.user)
        log("NovaShell removed.")
        return

    log(f"NovaShell setup ({'user' if args.user else 'system'})")
    check_rust()
    check_dev_headers()
    build()
    install_files(user=args.user)
    print()
    log("Installed.")
    print("    Test once now:   novashell --windowed")
    print("    Autostart next login:")
    print("        systemctl --user enable novashell")
    print("        systemctl --user start novashell")
    print("    Launch from GNOME:  Activities -> NovaShell")
    print("    Revert:             python3 setup.py --uninstall")


if __name__ == "__main__":
    main()