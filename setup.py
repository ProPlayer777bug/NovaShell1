
#!/usr/bin/env python3
"""NovaShell setup: install Rust, system deps, then build and install.

Usage:
    python3 setup.py                # full: deps + rust + build + install
    python3 setup.py --user         # per-user install (deps still system-wide)
    python3 setup.py --check        # inspect-only, touches nothing
    python3 setup.py --uninstall    # remove a previous install

Bootstrap (runs automatically if missing):
    - installs rustup + stable Rust via https://sh.rustup.rs
    - installs apt build packages:
        build-essential pkg-config libgtk-4-dev libwebkitgtk-6.0-dev
        libjavascriptcoregtk-6.0-dev libudev-dev

Safe by design:
    - never modifies GRUB, kernel params, initramfs, /etc/fstab,
      display managers, or GNOME configuration
    - GNOME stays installed and reachable as the desktop fallback
    - only writes to standard app / icon / autostart / systemd locations

Requires: python3 >= 3.8, curl (installed automatically), sudo for apt.
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


# Runtime helpers used opportunistically.
# The UI should still work if these are absent.
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


CARGO_HOME = Path(
    os.environ.get(
        "CARGO_HOME",
        str(Path.home() / ".cargo"),
    )
)

CARGO_BIN = CARGO_HOME / "bin"
CARGO = CARGO_BIN / "cargo"


def log(msg):
    print("==> {}".format(msg), flush=True)


def warn(msg):
    print("    WARNING: {}".format(msg), flush=True)


def run(cmd, **kw):
    """Run a command without automatically raising on failure."""
    return subprocess.run(cmd, check=False, **kw)


def which(binary):
    """Return the path to a binary, or None."""
    path = shutil.which(binary)
    return Path(path) if path else None


def is_deb_based():
    """Return True for Debian/Ubuntu-style systems."""
    os_release = Path("/etc/os-release")

    try:
        text = os_release.read_text(encoding="utf-8")
    except (OSError, UnicodeError):
        text = ""

    return (
        os.path.exists("/etc/debian_version")
        or "debian" in text.lower()
        or "ubuntu" in text.lower()
    )


def project_root():
    """Repo root = the folder containing this setup.py."""
    return Path(__file__).resolve().parent


def ensure_on_path():
    """Make ~/.cargo/bin visible to this process and its children."""
    bin_dir = str(CARGO_BIN)
    current_path = os.environ.get("PATH", "")

    if bin_dir not in current_path.split(os.pathsep):
        os.environ["PATH"] = bin_dir + os.pathsep + current_path


def ensure_rust():
    """Install rustup + stable Rust if cargo is missing."""
    ensure_on_path()

    cargo = which("cargo")

    if cargo is not None:
        rustc = which("rustc")

        if rustc is not None:
            result = subprocess.run(
                [str(rustc), "--version"],
                capture_output=True,
                text=True,
                check=False,
            )
            version = result.stdout.strip()
            log("rustc found: {}".format(version))
        else:
            log("cargo found, but rustc was not found.")

        return

    rustup = which("rustup")

    if rustup is not None:
        log("rustup present; installing/activating stable toolchain")

        result = run(
            [str(rustup), "default", "stable"],
            cwd=str(project_root()),
        )

        if result.returncode != 0:
            print("    rustup default failed. Run manually and retry.")
            sys.exit(1)

        ensure_on_path()

        if which("cargo") is None:
            print("    rustup completed but cargo is still unavailable.")
            sys.exit(1)

        return

    log(
        "Rust not found; installing rustup + stable "
        "(curl | sh -s -- -y)"
    )

    curl = which("curl")

    if curl is None:
        print("    curl is required to install rustup.")
        print("    Install it first:")
        print("        sudo apt update && sudo apt install -y curl")
        sys.exit(1)

    # IMPORTANT:
    # Keep the URL as a plain URL, not Markdown.
    command = (
        "{} --proto '=https' --tlsv1.2 -sSf "
        "https://sh.rustup.rs | sh -s -- -y"
    ).format(str(curl))

    result = subprocess.run(
        command,
        shell=True,
        text=True,
        check=False,
    )

    if result.returncode != 0:
        print(
            "    rustup install failed (exit {}). "
            "See output above.".format(result.returncode)
        )
        sys.exit(1)

    ensure_on_path()

    if not CARGO.exists():
        print(
            "    rustup reported success but cargo is missing from {}"
            .format(CARGO_BIN)
        )
        sys.exit(1)

    result = subprocess.run(
        [str(CARGO), "--version"],
        capture_output=True,
        text=True,
        check=False,
    )

    version = result.stdout.strip()

    if not version:
        print("    Could not determine cargo version.")
        sys.exit(1)

    log("installed: {}".format(version))


def require_sudo():
    """Ensure sudo is available and obtain credentials if necessary."""
    if os.geteuid() == 0:
        return

    if which("sudo") is None:
        print("    sudo is required for system package installation.")
        sys.exit(1)

    result = run(["sudo", "-n", "true"])

    if result.returncode != 0:
        print("    sudo password needed for system packages.")
        run(["sudo", "true"], check=True)


def ensure_system_deps():
    """Install BUILD_DEPS with apt on Debian/Ubuntu systems."""
    if not is_deb_based():
        warn("Not a Debian/Ubuntu OS; install these yourself:")
        print("        " + " ".join(BUILD_DEPS))
        return

    if which("apt-get") is None:
        warn("apt-get not found; skipping system dependency install.")
        return

    require_sudo()

    log("installing system packages (apt)")

    if os.geteuid() == 0:
        cmd = []
    else:
        cmd = ["sudo"]

    log("apt update")

    result = run(cmd + ["apt-get", "update"])

    if result.returncode != 0:
        print("    apt update failed; cannot continue.")
        sys.exit(1)

    log("apt install: " + " ".join(BUILD_DEPS))

    result = run(
        cmd + ["apt-get", "install", "-y"] + BUILD_DEPS
    )

    if result.returncode != 0:
        print("    apt install failed; cannot continue.")
        sys.exit(1)


def check_dev_headers():
    """Verify required development packages are available."""
    if which("pkg-config") is None:
        print(
            "    pkg-config not found; rerun setup "
            "(system deps step installs it)."
        )
        sys.exit(1)

    missing = []

    packages = (
        "gtk4",
        "javascriptcoregtk-6.0",
        "webkitgtk-6.0",
    )

    for package in packages:
        result = run(["pkg-config", "--exists", package])

        if result.returncode != 0:
            missing.append(package)

    if missing:
        print(
            "    missing dev packages: {}".format(
                ", ".join(missing)
            )
        )
        print(
            "        Re-run: python3 setup.py "
            "(installs them via apt)"
        )
        sys.exit(1)

    for package in packages:
        result = subprocess.run(
            ["pkg-config", "--modversion", package],
            capture_output=True,
            text=True,
            check=False,
        )

        version = result.stdout.strip()

        print(
            "    found: {} {}".format(
                package,
                version,
            )
        )


def get_rust_version():
    """
    Return (major, minor, patch) for rustc, or None.

    This function only inspects the system.
    """
    rustc = which("rustc")

    if rustc is None:
        return None

    result = subprocess.run(
        [str(rustc), "--version"],
        capture_output=True,
        text=True,
        check=False,
    )

    version = result.stdout.strip()

    import re

    match = re.search(
        r"rustc (\d+)\.(\d+)\.(\d+)",
        version,
    )

    if not match:
        return None

    return (
        int(match.group(1)),
        int(match.group(2)),
        int(match.group(3)),
    )


def check_rust_version(update_if_needed=True):
    """
    Enforce rustc >= 1.85.

    If update_if_needed=False, this function never modifies anything.
    """
    rustc = which("rustc")

    if rustc is None:
        print(
            "    rustc not found. Run: "
            "python3 setup.py"
        )
        sys.exit(1)

    result = subprocess.run(
        [str(rustc), "--version"],
        capture_output=True,
        text=True,
        check=False,
    )

    version = result.stdout.strip()

    import re

    match = re.search(
        r"rustc (\d+)\.(\d+)",
        version,
    )

    if not match:
        print("    cannot parse rustc version: {}".format(version))
        sys.exit(1)

    major = int(match.group(1))
    minor = int(match.group(2))

    if major > 1 or (major == 1 and minor >= 85):
        print("    rustc OK: {}".format(version))
        return True

    print(
        "    rustc {}.{} is too old (need >= 1.85)."
        .format(major, minor)
    )

    if not update_if_needed:
        print("    --check mode: no update performed.")
        return False

    rustup = which("rustup")

    if rustup is None:
        print(
            "    rustup is not installed, so Rust cannot be updated."
        )
        sys.exit(1)

    print("    updating stable Rust toolchain...")

    result = run(
        [str(rustup), "update", "stable"]
    )

    if result.returncode != 0:
        print("    rustup update stable failed.")
        sys.exit(1)

    ensure_on_path()

    # Check again after the update.
    new_version = get_rust_version()

    if new_version is None:
        print("    unable to verify rustc after update.")
        sys.exit(1)

    new_major, new_minor, _ = new_version

    if not (
        new_major > 1
        or (new_major == 1 and new_minor >= 85)
    ):
        print(
            "    Rust was updated but is still too old."
        )
        sys.exit(1)

    return True


def build():
    """Build the release binary."""
    log("building release binary (cargo build --release)")

    ensure_on_path()

    cargo = which("cargo")

    if cargo is None:
        print("    cargo not found.")
        sys.exit(1)

    result = run(
        [str(cargo), "build", "--release"],
        cwd=str(project_root()),
    )

    if result.returncode != 0:
        print("    build failed. See output above.")
        sys.exit(1)

    bin_path = (
        project_root()
        / "target"
        / "release"
        / BIN_NAME
    )

    if not bin_path.exists():
        print(
            "    expected binary not found: {}".format(
                bin_path
            )
        )
        sys.exit(1)

    if not os.access(str(bin_path), os.X_OK):
        print(
            "    built binary is not executable: {}".format(
                bin_path
            )
        )
        sys.exit(1)


def get_install_dirs(user):
    """Return installation directories for user/system installation."""
    home = Path.home()

    if user:
        bin_dir = Path(
            os.environ.get(
                "NOVA_BIN_DIR",
                str(home / ".local" / "bin"),
            )
        )

        apps_dir = (
            home
            / ".local"
            / "share"
            / "applications"
        )

        icons_dir = (
            home
            / ".local"
            / "share"
            / "icons"
            / "hicolor"
            / "scalable"
            / "apps"
        )

        autostart_dir = (
            home
            / ".config"
            / "autostart"
        )

        systemd_dir = (
            home
            / ".config"
            / "systemd"
            / "user"
        )

    else:
        if os.geteuid() != 0:
            print(
                "System-wide install needs root."
            )
            print(
                "Re-run: sudo python3 setup.py"
            )
            print(
                "Or use: python3 setup.py --user"
            )
            sys.exit(1)

        bin_dir = Path("/usr/local/bin")

        apps_dir = Path(
            "/usr/local/share/applications"
        )

        icons_dir = Path(
            "/usr/local/share/icons/"
            "hicolor/scalable/apps"
        )

        autostart_dir = Path(
            "/etc/xdg/autostart"
        )

        systemd_dir = Path(
            "/etc/systemd/user"
        )

    return (
        bin_dir,
        apps_dir,
        icons_dir,
        autostart_dir,
        systemd_dir,
    )


def install_files(user):
    """Install the NovaShell files."""
    (
        bin_dir,
        apps_dir,
        icons_dir,
        autostart_dir,
        systemd_dir,
    ) = get_install_dirs(user)

    for directory in (
        bin_dir,
        apps_dir,
        icons_dir,
        autostart_dir,
        systemd_dir,
    ):
        directory.mkdir(
            parents=True,
            exist_ok=True,
        )

    root = project_root()

    binary = (
        root
        / "target"
        / "release"
        / BIN_NAME
    )

    icon = root / ICON_SRC
    desktop = root / DESKTOP_SRC
    autostart_source = root / AUTOSTART_SRC
    service_source = root / SERVICE_SRC

    # Validate source files before modifying the installation.
    required_files = [
        binary,
        icon,
        desktop,
        autostart_source,
        service_source,
    ]

    for source in required_files:
        if not source.exists():
            print(
                "    required file missing: {}".format(
                    source
                )
            )
            sys.exit(1)

    installed_binary = bin_dir / BIN_NAME
    installed_desktop = (
        apps_dir / "novashell.desktop"
    )
    installed_icon = (
        icons_dir / "org.novashell.svg"
    )
    installed_autostart = (
        autostart_dir / "novashell.desktop"
    )
    installed_service = (
        systemd_dir / "novashell.service"
    )

    shutil.copy2(
        binary,
        installed_binary,
    )

    os.chmod(
        str(installed_binary),
        0o755,
    )

    shutil.copy2(
        icon,
        installed_icon,
    )

    shutil.copy2(
        desktop,
        installed_desktop,
    )

    autostart = autostart_source.read_text(
        encoding="utf-8"
    )

    autostart = autostart.replace(
        "@BINARY@",
        str(installed_binary),
    )

    installed_autostart.write_text(
        autostart,
        encoding="utf-8",
    )

    service = service_source.read_text(
        encoding="utf-8"
    )

    service = service.replace(
        "@BINARY@",
        str(installed_binary),
    )

    installed_service.write_text(
        service,
        encoding="utf-8",
    )

    log("installed files:")

    for file_path in (
        installed_binary,
        installed_desktop,
        installed_icon,
        installed_autostart,
        installed_service,
    ):
        print("    {}".format(file_path))

    # The icon cache belongs to the hicolor directory.
    gtk_update = which("gtk-update-icon-cache")

    if gtk_update is not None:
        hicolor_dir = icons_dir.parent.parent

        run(
            [
                str(gtk_update),
                "-f",
                "-t",
                str(hicolor_dir),
            ]
        )

    # Only reload the current user's systemd user manager.
    systemctl = which("systemctl")

    if systemctl is not None:
        run(
            [
                str(systemctl),
                "--user",
                "daemon-reload",
            ]
        )


def uninstall(user):
    """Remove a previous NovaShell installation."""
    (
        bin_dir,
        apps_dir,
        icons_dir,
        autostart_dir,
        systemd_dir,
    ) = get_install_dirs(user)

    files = (
        bin_dir / BIN_NAME,
        apps_dir / "novashell.desktop",
        icons_dir / "org.novashell.svg",
        autostart_dir / "novashell.desktop",
        systemd_dir / "novashell.service",
    )

    systemctl = which("systemctl")

    # Stop/disable the service only if this is the current
    # user's user service. This does not touch system services.
    if systemctl is not None:
        service_name = "novashell.service"

        run(
            [
                str(systemctl),
                "--user",
                "disable",
                "--now",
                service_name,
            ]
        )

    for file_path in files:
        if file_path.exists():
            try:
                file_path.unlink()
                print(
                    "    removed {}".format(
                        file_path
                    )
                )
            except OSError as exc:
                warn(
                    "could not remove {}: {}".format(
                        file_path,
                        exc,
                    )
                )

    if systemctl is not None:
        run(
            [
                str(systemctl),
                "--user",
                "daemon-reload",
            ]
        )


def check_mode():
    """
    Inspect the environment without modifying anything.

    IMPORTANT:
    This function must never install/update anything.
    """
    log("checking environment (no changes made)")

    ensure_on_path()

    # Check Rust only; do NOT update it.
    rust_ok = check_rust_version(
        update_if_needed=False
    )

    check_dev_headers()

    for tool, description in RUNTIME_HINTS:
        status = "OK " if which(tool) else "---"

        print(
            "    {} {:16s} {}".format(
                status,
                tool,
                description,
            )
        )

    if not rust_ok:
        print(
            "    NOTE: Rust >= 1.85 is required for building."
        )

    print("    Environment check done.")


def main():
    parser = argparse.ArgumentParser(
        description="Bootstrap, build and install NovaShell"
    )

    parser.add_argument(
        "--user",
        action="store_true",
        help="per-user install",
    )

    parser.add_argument(
        "--check",
        action="store_true",
        help="verify only, change nothing",
    )

    parser.add_argument(
        "--uninstall",
        action="store_true",
        help="remove NovaShell",
    )

    args = parser.parse_args()

    if platform.system() != "Linux":
        print(
            "NovaShell targets Ubuntu/Debian Linux. "
            "This looks like: {}".format(
                platform.system()
            )
        )
        sys.exit(1)

    if args.check:
        check_mode()
        return

    if args.uninstall:
        uninstall(user=args.user)
        log("NovaShell removed.")
        return

    log(
        "NovaShell setup ({})".format(
            "user" if args.user else "system"
        )
    )

    ensure_rust()

    ensure_system_deps()

    check_rust_version(
        update_if_needed=True
    )

    check_dev_headers()

    build()

    install_files(
        user=args.user
    )

    print()

    log("Installed.")

    print(
        "    Test once now:   novashell --windowed"
    )

    print(
        "    Autostart next login:"
    )

    print(
        "        systemctl --user enable novashell"
    )

    print(
        "        systemctl --user start novashell"
    )

    print(
        "    Launch from GNOME:  Activities -> NovaShell"
    )

    print(
        "    Revert:"
    )

    print(
        "        python3 setup.py --uninstall"
    )


if __name__ == "__main__":
    main()