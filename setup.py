#!/usr/bin/env python3
"""
NovaShell setup: install dependencies, Rust, build, and install.

Usage:
    python3 setup.py
    python3 setup.py --user
    python3 setup.py --check
    python3 setup.py --uninstall

Examples:
    # System-wide installation
    sudo python3 setup.py

    # Per-user installation
    python3 setup.py --user

    # Inspect only; makes no changes
    python3 setup.py --check

    # Remove system installation
    sudo python3 setup.py --uninstall

    # Remove user installation
    python3 setup.py --user --uninstall


Compatibility:
    - Linux
    - Debian/Ubuntu-style systems are supported automatically
    - Python >= 3.8
    - Rust >= 1.85
    - GTK >= 4.14
    - WebKitGTK 6.0 development files

Important:
    This script does NOT modify:
        - GRUB
        - kernel parameters
        - initramfs
        - /etc/fstab
        - display managers
        - GNOME settings

It only installs NovaShell application files and required build packages.
"""


import argparse
import os
import platform
import re
import shutil
import subprocess
import sys
from pathlib import Path


# ---------------------------------------------------------------------------
# Project configuration
# ---------------------------------------------------------------------------

APP_NAME = "novashell"
BIN_NAME = "novashell"

ICON_SRC = "assets/org.novashell.svg"
DESKTOP_SRC = "data/novashell.desktop"
AUTOSTART_SRC = "data/autostart/novashell.desktop"
SERVICE_SRC = "data/systemd/novashell.service"


# Ubuntu 24.04 provides GTK 4.14.x.
# NovaShell's Rust dependencies should therefore use:
#
#     gtk4 = { version = "0.10", features = ["v4_14"] }
#     webkit6 = "0.5"
#
MIN_GTK_MAJOR = 4
MIN_GTK_MINOR = 14


# Rust version required by the project.
MIN_RUST_MAJOR = 1
MIN_RUST_MINOR = 85


BUILD_DEPS = [
    "curl",
    "build-essential",
    "pkg-config",
    "libgtk-4-dev",
    "libwebkitgtk-6.0-dev",
    "libjavascriptcoregtk-6.0-dev",
    "libudev-dev",
]


RUNTIME_HINTS = [
    ("wpctl", "volume control (WirePlumber/PipeWire)"),
    ("brightnessctl", "backlight control"),
    ("grim", "Wayland screenshots"),
    ("gnome-screenshot", "X11 screenshots"),
    ("nmcli", "Wi-Fi SSID in status bar"),
    ("bluetoothctl", "Bluetooth device list"),
    ("systemctl", "power/service actions"),
    ("gnome-session-quit", "log out action"),
]


# ---------------------------------------------------------------------------
# Cargo locations
# ---------------------------------------------------------------------------

CARGO_HOME = Path(
    os.environ.get(
        "CARGO_HOME",
        str(Path.home() / ".cargo"),
    )
)

CARGO_BIN = CARGO_HOME / "bin"
CARGO = CARGO_BIN / "cargo"
RUSTC = CARGO_BIN / "rustc"
RUSTUP = CARGO_BIN / "rustup"


# ---------------------------------------------------------------------------
# Generic helpers
# ---------------------------------------------------------------------------

def log(message):
    print("==> {}".format(message), flush=True)


def info(message):
    print("    {}".format(message), flush=True)


def warn(message):
    print("    WARNING: {}".format(message), flush=True)


def error(message):
    print("    ERROR: {}".format(message), flush=True)


def run(command, **kwargs):
    """
    Run a command without automatically raising.

    Returns subprocess.CompletedProcess.
    """
    return subprocess.run(
        command,
        check=False,
        **kwargs
    )


def command_exists(name):
    """Return True if an executable exists in PATH."""
    return shutil.which(name) is not None


def command_path(name):
    """Return Path to executable or None."""
    path = shutil.which(name)
    return Path(path) if path else None


def project_root():
    """Return the directory containing this setup.py."""
    return Path(__file__).resolve().parent


def is_root():
    """Return whether the current process has UID 0."""
    return hasattr(os, "geteuid") and os.geteuid() == 0


def python_version_ok():
    """Check Python >= 3.8."""
    return sys.version_info >= (3, 8)


def ensure_python_version():
    if python_version_ok():
        return

    error(
        "Python 3.8 or newer is required. "
        "Detected {}.{}.{}.".format(
            sys.version_info.major,
            sys.version_info.minor,
            sys.version_info.micro,
        )
    )
    sys.exit(1)


# ---------------------------------------------------------------------------
# OS detection
# ---------------------------------------------------------------------------

def read_os_release():
    """Read /etc/os-release into a dictionary."""
    path = Path("/etc/os-release")

    if not path.exists():
        return {}

    result = {}

    try:
        for line in path.read_text(
            encoding="utf-8"
        ).splitlines():
            line = line.strip()

            if not line or "=" not in line:
                continue

            key, value = line.split(
                "=",
                1,
            )

            value = value.strip().strip('"')

            result[key] = value

    except (OSError, UnicodeError):
        return {}

    return result


def is_debian_based():
    """Return True for Debian/Ubuntu-like systems."""
    data = read_os_release()

    if Path("/etc/debian_version").exists():
        return True

    ids = {
        data.get("ID", "").lower(),
        *[
            item.strip().lower()
            for item in data.get(
                "ID_LIKE",
                "",
            ).split()
        ],
    }

    return bool(
        {"debian", "ubuntu"} & ids
    )


def is_ubuntu():
    data = read_os_release()

    return data.get(
        "ID",
        "",
    ).lower() == "ubuntu"


def print_os_info():
    data = read_os_release()

    if not data:
        info(
            "OS: unable to read /etc/os-release"
        )
        return

    info(
        "OS: {} {}".format(
            data.get("NAME", "unknown"),
            data.get("VERSION_ID", ""),
        ).strip()
    )

    if data.get("VERSION_CODENAME"):
        info(
            "Codename: {}".format(
                data["VERSION_CODENAME"]
            )
        )


def ensure_linux():
    if platform.system() != "Linux":
        error(
            "NovaShell targets Linux. "
            "Detected: {}".format(
                platform.system()
            )
        )
        sys.exit(1)


# ---------------------------------------------------------------------------
# Project validation
# ---------------------------------------------------------------------------

def required_project_files():
    root = project_root()

    return [
        root / "Cargo.toml",
        root / "src",
        root / ICON_SRC,
        root / DESKTOP_SRC,
        root / AUTOSTART_SRC,
        root / SERVICE_SRC,
    ]


def validate_project_files():
    """
    Verify the repository is complete before making changes.
    """
    root = project_root()

    if not root.exists():
        error(
            "Project root does not exist: {}".format(
                root
            )
        )
        sys.exit(1)

    missing = []

    for path in required_project_files():
        if not path.exists():
            missing.append(path)

    if missing:
        error("Required NovaShell files are missing:")

        for path in missing:
            print(
                "        {}".format(path)
            )

        sys.exit(1)


# ---------------------------------------------------------------------------
# PATH / Rust
# ---------------------------------------------------------------------------

def ensure_cargo_path():
    """
    Make ~/.cargo/bin available to this process and child processes.
    """
    cargo_bin = str(CARGO_BIN)

    current = os.environ.get(
        "PATH",
        "",
    )

    parts = current.split(
        os.pathsep
    )

    if cargo_bin not in parts:
        os.environ["PATH"] = (
            cargo_bin
            + os.pathsep
            + current
        )


def rustc_version():
    """
    Return (major, minor, patch) or None.
    """
    rustc = command_path("rustc")

    if rustc is None:
        return None

    result = subprocess.run(
        [
            str(rustc),
            "--version",
        ],
        capture_output=True,
        text=True,
        check=False,
    )

    text = result.stdout.strip()

    match = re.search(
        r"rustc\s+(\d+)\.(\d+)\.(\d+)",
        text,
    )

    if not match:
        return None

    return (
        int(match.group(1)),
        int(match.group(2)),
        int(match.group(3)),
    )


def rust_version_string():
    rustc = command_path("rustc")

    if rustc is None:
        return None

    result = subprocess.run(
        [
            str(rustc),
            "--version",
        ],
        capture_output=True,
        text=True,
        check=False,
    )

    return result.stdout.strip()


def rust_version_satisfies(version):
    if version is None:
        return False

    major, minor, _ = version

    return (
        major > MIN_RUST_MAJOR
        or (
            major == MIN_RUST_MAJOR
            and minor >= MIN_RUST_MINOR
        )
    )


def ensure_rust():
    """
    Ensure Rust >= 1.85 exists.

    This may install/update Rust during normal installation.
    """
    ensure_cargo_path()

    version = rustc_version()

    if rust_version_satisfies(version):
        log(
            "Rust OK: {}".format(
                rust_version_string()
            )
        )
        return

    if version is not None:
        warn(
            "Rust {}.{} is too old; "
            "Rust >= {}.{} is required.".format(
                version[0],
                version[1],
                MIN_RUST_MAJOR,
                MIN_RUST_MINOR,
            )
        )

    rustup = command_path("rustup")

    if rustup is not None:
        log(
            "Updating/activating stable Rust toolchain"
        )

        result = run(
            [
                str(rustup),
                "toolchain",
                "install",
                "stable",
            ]
        )

        if result.returncode != 0:
            error(
                "Unable to install the stable Rust toolchain."
            )
            sys.exit(1)

        result = run(
            [
                str(rustup),
                "default",
                "stable",
            ]
        )

        if result.returncode != 0:
            error(
                "Unable to select stable Rust."
            )
            sys.exit(1)

        ensure_cargo_path()

        version = rustc_version()

        if rust_version_satisfies(version):
            log(
                "Rust ready: {}".format(
                    rust_version_string()
                )
            )
            return

        error(
            "Rust was installed/updated, "
            "but the required version is still unavailable."
        )
        sys.exit(1)

    # Rustup is absent. Install it.
    curl = command_path("curl")

    if curl is None:
        error(
            "curl is required to install Rust."
        )
        print(
            "        sudo apt update"
        )
        print(
            "        sudo apt install -y curl"
        )
        sys.exit(1)

    log(
        "Rust not found; installing rustup"
    )

    command = (
        "{} --proto '=https' "
        "--tlsv1.2 -sSf "
        "https://sh.rustup.rs "
        "| sh -s -- -y"
    ).format(
        str(curl)
    )

    result = subprocess.run(
        command,
        shell=True,
        executable="/bin/bash",
        text=True,
        check=False,
    )

    if result.returncode != 0:
        error(
            "rustup installation failed."
        )
        sys.exit(1)

    ensure_cargo_path()

    version = rustc_version()

    if not rust_version_satisfies(version):
        error(
            "rustup installed, but Rust >= {}.{} "
            "could not be verified.".format(
                MIN_RUST_MAJOR,
                MIN_RUST_MINOR,
            )
        )
        sys.exit(1)

    log(
        "Rust installed: {}".format(
            rust_version_string()
        )
    )


# ---------------------------------------------------------------------------
# sudo / apt
# ---------------------------------------------------------------------------

def ensure_sudo():
    """
    Make sure sudo is available for non-root system operations.
    """
    if is_root():
        return

    if not command_exists("sudo"):
        error(
            "sudo is required for system package installation."
        )
        sys.exit(1)

    result = run(
        [
            "sudo",
            "-n",
            "true",
        ]
    )

    if result.returncode == 0:
        return

    info(
        "sudo password is required."
    )

    result = run(
        [
            "sudo",
            "true",
        ]
    )

    if result.returncode != 0:
        error(
            "Unable to obtain sudo privileges."
        )
        sys.exit(1)


def apt_command():
    """
    Return the correct apt command prefix.
    """
    if is_root():
        return []

    return ["sudo"]


def apt_install_packages(packages):
    """
    Install packages using apt-get.
    """
    ensure_sudo()

    if not command_exists("apt-get"):
        error(
            "apt-get was not found."
        )
        sys.exit(1)

    prefix = apt_command()

    log("Updating apt package lists")

    result = run(
        prefix
        + [
            "apt-get",
            "update",
        ]
    )

    if result.returncode != 0:
        error(
            "apt-get update failed."
        )
        sys.exit(1)

    log(
        "Installing system dependencies"
    )

    info(
        "Packages: {}".format(
            " ".join(packages)
        )
    )

    result = run(
        prefix
        + [
            "apt-get",
            "install",
            "-y",
        ]
        + packages
    )

    if result.returncode != 0:
        error(
            "apt-get install failed."
        )
        sys.exit(1)


def ensure_system_deps():
    """
    Install Debian/Ubuntu build dependencies.
    """
    if not is_debian_based():
        warn(
            "This is not a Debian/Ubuntu-style system."
        )

        print(
            "    Install the following packages manually:"
        )

        print(
            "        {}".format(
                " ".join(BUILD_DEPS)
            )
        )

        return

    if not command_exists("apt-get"):
        error(
            "apt-get is unavailable."
        )
        sys.exit(1)

    apt_install_packages(
        BUILD_DEPS
    )


# ---------------------------------------------------------------------------
# pkg-config / GTK / WebKit checks
# ---------------------------------------------------------------------------

def pkg_config_exists(package):
    result = run(
        [
            "pkg-config",
            "--exists",
            package,
        ]
    )

    return result.returncode == 0


def pkg_config_version(package):
    result = subprocess.run(
        [
            "pkg-config",
            "--modversion",
            package,
        ],
        capture_output=True,
        text=True,
        check=False,
    )

    if result.returncode != 0:
        return None

    value = result.stdout.strip()

    return value or None


def parse_version(value):
    """
    Parse a version beginning with MAJOR.MINOR.PATCH.

    Example:
        4.14.5+ds -> (4, 14, 5)
    """
    if not value:
        return None

    match = re.match(
        r"^\s*(\d+)\.(\d+)(?:\.(\d+))?",
        value,
    )

    if not match:
        return None

    return (
        int(match.group(1)),
        int(match.group(2)),
        int(match.group(3) or 0),
    )


def version_at_least(
    actual,
    required,
):
    if actual is None:
        return False

    return actual >= required


def check_pkg_config():
    if not command_exists("pkg-config"):
        error(
            "pkg-config is not installed."
        )
        print(
            "    Run:"
        )
        print(
            "        sudo apt install -y pkg-config"
        )
        sys.exit(1)


def check_dev_headers():
    """
    Verify GTK 4.14+ and WebKitGTK 6.0 development packages.
    """
    check_pkg_config()

    required = [
        "gtk4",
        "webkitgtk-6.0",
        "javascriptcoregtk-6.0",
    ]

    missing = []

    for package in required:
        if not pkg_config_exists(package):
            missing.append(package)

    if missing:
        error(
            "Missing development packages:"
        )

        for package in missing:
            print(
                "        {}".format(package)
            )

        print()
        print(
            "    Run:"
        )
        print(
            "        sudo apt install "
            + " ".join(BUILD_DEPS)
        )

        sys.exit(1)

    versions = {}

    for package in required:
        version = pkg_config_version(
            package
        )

        versions[package] = version

        info(
            "{} {}".format(
                package,
                version or "unknown",
            )
        )

    gtk_version_text = versions["gtk4"]
    gtk_version = parse_version(
        gtk_version_text
    )

    if gtk_version is None:
        error(
            "Unable to parse GTK version: {}".format(
                gtk_version_text
            )
        )
        sys.exit(1)

    required_gtk = (
        MIN_GTK_MAJOR,
        MIN_GTK_MINOR,
        0,
    )

    if not version_at_least(
        gtk_version,
        required_gtk,
    ):
        print()
        error(
            "GTK {} is too old.".format(
                gtk_version_text
            )
        )

        print(
            "    NovaShell requires GTK >= {}.{}.".format(
                MIN_GTK_MAJOR,
                MIN_GTK_MINOR,
            )
        )

        print(
            "    Your system has GTK {}.".format(
                gtk_version_text
            )
        )

        print()
        print(
            "    Do NOT try to solve this with "
            "PKG_CONFIG_PATH."
        )

        print(
            "    The installed GTK itself is too old."
        )

        print(
            "    For Ubuntu 24.04, use the GTK-compatible "
            "NovaShell dependencies:"
        )

        print(
            '        gtk4 = { version = "0.10", features = ["v4_14"] }'
        )

        print(
            '        webkit6 = "0.5"'
        )

        sys.exit(1)

    info(
        "GTK version requirement: OK"
    )


# ---------------------------------------------------------------------------
# Cargo dependency sanity checks
# ---------------------------------------------------------------------------

def read_cargo_toml():
    """
    Read Cargo.toml as plain text.

    We intentionally don't require a TOML parser because setup.py
    should work on a minimal Python installation.
    """
    path = project_root() / "Cargo.toml"

    try:
        return path.read_text(
            encoding="utf-8"
        )
    except OSError as exc:
        error(
            "Unable to read Cargo.toml: {}".format(
                exc
            )
        )
        sys.exit(1)


def check_cargo_dependency_configuration():
    """
    Detect the common GTK 4.21/4.22 configuration that caused
    the previous build failure.
    """
    text = read_cargo_toml()

    gtk_match = re.search(
        r'(?m)^\s*gtk4\s*=\s*(.+)$',
        text,
    )

    webkit_match = re.search(
        r'(?m)^\s*webkit6\s*=\s*(.+)$',
        text,
    )

    if gtk_match:
        gtk_line = gtk_match.group(1).strip()

        if (
            "version = \"0.11\"" in gtk_line
            or "version=\"0.11\"" in gtk_line
            or '"0.11"' in gtk_line
        ):
            warn(
                "Cargo.toml appears to use gtk4 0.11."
            )

            warn(
                "That can require a newer system GTK than Ubuntu 24.04 provides."
            )

            warn(
                'For GTK 4.14 systems, use: '
                'gtk4 = { version = "0.10", features = ["v4_14"] }'
            )

        if "v4_22" in gtk_line:
            warn(
                "Cargo.toml enables the GTK v4_22 API."
            )

            warn(
                "That is not appropriate for a GTK 4.14 system."
            )

    if webkit_match:
        webkit_line = webkit_match.group(1).strip()

        if (
            '"0.6"' in webkit_line
            or "version = \"0.6\"" in webkit_line
        ):
            warn(
                "Cargo.toml appears to use webkit6 0.6."
            )

            warn(
                'For the GTK 0.10 dependency generation, use webkit6 = "0.5".'
            )


# ---------------------------------------------------------------------------
# Cargo environment / build
# ---------------------------------------------------------------------------

def cargo_path():
    cargo = command_path("cargo")

    if cargo is None:
        return None

    return str(cargo)


def cargo_version():
    cargo = cargo_path()

    if cargo is None:
        return None

    result = subprocess.run(
        [
            cargo,
            "--version",
        ],
        capture_output=True,
        text=True,
        check=False,
    )

    return result.stdout.strip()


def ensure_cargo_available():
    ensure_cargo_path()

    if cargo_path() is None:
        error(
            "cargo is not available after Rust setup."
        )
        sys.exit(1)


def cargo_tree_summary():
    """
    Print GTK/WebKit dependencies if cargo tree is available.

    This is informational only.
    """
    cargo = cargo_path()

    if cargo is None:
        return

    result = subprocess.run(
        [
            cargo,
            "tree",
            "--depth",
            "2",
        ],
        cwd=str(project_root()),
        capture_output=True,
        text=True,
        check=False,
    )

    if result.returncode != 0:
        warn(
            "Could not inspect Cargo dependency tree."
        )
        return

    lines = []

    for line in result.stdout.splitlines():
        if any(
            name in line
            for name in (
                "gtk4 ",
                "gtk4-sys ",
                "gdk4 ",
                "gdk4-sys ",
                "gsk4 ",
                "gsk4-sys ",
                "webkit6 ",
                "webkit6-sys ",
            )
        ):
            lines.append(line)

    if not lines:
        return

    print()
    info("GTK/WebKit Cargo dependencies:")

    for line in lines:
        print(
            "        {}".format(
                line.strip()
            )
        )


def cargo_build():
    """
    Build the release binary.

    Uses the existing Cargo.lock if present.
    Cargo itself updates the lock file when Cargo.toml
    requires dependency resolution changes.
    """
    ensure_cargo_available()

    cargo = cargo_path()

    log(
        "Building NovaShell in release mode"
    )

    info(
        "Command: cargo build --release"
    )

    result = run(
        [
            cargo,
            "build",
            "--release",
        ],
        cwd=str(project_root()),
    )

    if result.returncode != 0:
        print()
        error(
            "Cargo build failed."
        )

        print()
        print(
            "    If this mentions:"
        )

        print(
            "        gtk4 >= 4.21 could not be satisfied"
        )

        print(
            "    verify Cargo.toml uses:"
        )

        print(
            '        gtk4 = { version = "0.10", features = ["v4_14"] }'
        )

        print(
            '        webkit6 = "0.5"'
        )

        sys.exit(1)


def verify_binary():
    binary = (
        project_root()
        / "target"
        / "release"
        / BIN_NAME
    )

    if not binary.exists():
        error(
            "Build completed but binary was not found:"
        )

        print(
            "        {}".format(binary)
        )

        sys.exit(1)

    if not binary.is_file():
        error(
            "Expected binary path is not a regular file:"
        )

        print(
            "        {}".format(binary)
        )

        sys.exit(1)

    if not os.access(
        str(binary),
        os.X_OK,
    ):
        error(
            "Built binary is not executable:"
        )

        print(
            "        {}".format(binary)
        )

        sys.exit(1)

    size = binary.stat().st_size

    if size == 0:
        error(
            "Built binary is empty."
        )
        sys.exit(1)

    info(
        "Binary: {} ({:.1f} MB)".format(
            binary,
            size / 1024 / 1024,
        )
    )

    return binary


# ---------------------------------------------------------------------------
# Installation paths
# ---------------------------------------------------------------------------

def get_install_dirs(user):
    """
    Return installation directories.

    User installation:
        ~/.local/bin
        ~/.local/share/applications
        ~/.local/share/icons/...
        ~/.config/autostart
        ~/.config/systemd/user

    System installation:
        /usr/local/bin
        /usr/local/share/applications
        /usr/local/share/icons/...
        /etc/xdg/autostart
        /etc/systemd/user
    """
    home = Path.home()

    if user:
        bin_dir = Path(
            os.environ.get(
                "NOVA_BIN_DIR",
                str(
                    home
                    / ".local"
                    / "bin"
                ),
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
        if not is_root():
            error(
                "System-wide installation requires root."
            )

            print()
            print(
                "    Use:"
            )

            print(
                "        sudo python3 setup.py"
            )

            print()
            print(
                "    Or use a user installation:"
            )

            print(
                "        python3 setup.py --user"
            )

            sys.exit(1)

        bin_dir = Path(
            "/usr/local/bin"
        )

        apps_dir = Path(
            "/usr/local/share/applications"
        )

        icons_dir = Path(
            "/usr/local/share/icons"
            "/hicolor/scalable/apps"
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


# ---------------------------------------------------------------------------
# File installation
# ---------------------------------------------------------------------------

def validate_install_sources():
    root = project_root()

    sources = {
        "binary": (
            root
            / "target"
            / "release"
            / BIN_NAME
        ),
        "icon": root / ICON_SRC,
        "desktop": root / DESKTOP_SRC,
        "autostart": root / AUTOSTART_SRC,
        "service": root / SERVICE_SRC,
    }

    missing = []

    for name, path in sources.items():
        if not path.exists():
            missing.append(
                "{}: {}".format(
                    name,
                    path,
                )
            )

    if missing:
        error(
            "Installation files are missing:"
        )

        for item in missing:
            print(
                "        {}".format(item)
            )

        sys.exit(1)

    return sources


def safe_mkdir(path):
    try:
        path.mkdir(
            parents=True,
            exist_ok=True,
        )
    except OSError as exc:
        error(
            "Unable to create directory {}: {}".format(
                path,
                exc,
            )
        )
        sys.exit(1)


def install_files(user):
    """
    Install NovaShell files.

    Files are copied only after all required source files
    have been validated.
    """
    sources = validate_install_sources()

    (
        bin_dir,
        apps_dir,
        icons_dir,
        autostart_dir,
        systemd_dir,
    ) = get_install_dirs(user)

    directories = (
        bin_dir,
        apps_dir,
        icons_dir,
        autostart_dir,
        systemd_dir,
    )

    for directory in directories:
        safe_mkdir(directory)

    installed_binary = (
        bin_dir / BIN_NAME
    )

    installed_desktop = (
        apps_dir / "novashell.desktop"
    )

    installed_icon = (
        icons_dir / "org.novashell.svg"
    )

    installed_autostart = (
        autostart_dir
        / "novashell.desktop"
    )

    installed_service = (
        systemd_dir
        / "novashell.service"
    )

    log(
        "Installing NovaShell files"
    )

    # Binary
    shutil.copy2(
        sources["binary"],
        installed_binary,
    )

    os.chmod(
        str(installed_binary),
        0o755,
    )

    # Icon
    shutil.copy2(
        sources["icon"],
        installed_icon,
    )

    # Desktop entry
    desktop_text = Path(
        sources["desktop"]
    ).read_text(
        encoding="utf-8"
    )

    desktop_text = desktop_text.replace(
        "@BINARY@",
        str(installed_binary),
    )

    installed_desktop.write_text(
        desktop_text,
        encoding="utf-8",
    )

    # Autostart entry
    autostart_text = Path(
        sources["autostart"]
    ).read_text(
        encoding="utf-8"
    )

    autostart_text = autostart_text.replace(
        "@BINARY@",
        str(installed_binary),
    )

    installed_autostart.write_text(
        autostart_text,
        encoding="utf-8",
    )

    # systemd user service
    service_text = Path(
        sources["service"]
    ).read_text(
        encoding="utf-8"
    )

    service_text = service_text.replace(
        "@BINARY@",
        str(installed_binary),
    )

    installed_service.write_text(
        service_text,
        encoding="utf-8",
    )

    print()
    log("Installed files:")

    for path in (
        installed_binary,
        installed_desktop,
        installed_icon,
        installed_autostart,
        installed_service,
    ):
        print(
            "    {}".format(path)
        )

    update_icon_cache(
        icons_dir
    )

    reload_systemd_user_manager(
        user=user
    )


def update_icon_cache(icons_dir):
    """
    Update the hicolor icon cache when possible.
    """
    executable = command_path(
        "gtk-update-icon-cache"
    )

    if executable is None:
        return

    # icons_dir:
    #   .../icons/hicolor/scalable/apps
    #
    # hicolor:
    #   icons_dir.parent.parent
    hicolor_dir = (
        icons_dir.parent.parent
    )

    run(
        [
            str(executable),
            "-f",
            "-t",
            str(hicolor_dir),
        ]
    )


def reload_systemd_user_manager(user):
    """
    Reload the current user's systemd user manager.

    For system-wide installation performed with sudo, do not
    blindly run `systemctl --user` as root because that targets
    root's user manager rather than the desktop user's manager.
    """
    executable = command_path(
        "systemctl"
    )

    if executable is None:
        return

    if not user:
        info(
            "System-wide service installed."
        )

        info(
            "Run as your desktop user:"
        )

        info(
            "systemctl --user daemon-reload"
        )

        return

    # User installation.
    result = run(
        [
            str(executable),
            "--user",
            "daemon-reload",
        ]
    )

    if result.returncode != 0:
        warn(
            "Could not reload the user systemd manager."
        )

        warn(
            "This is usually harmless if the session "
            "does not currently have a user systemd bus."
        )


# ---------------------------------------------------------------------------
# Uninstall
# ---------------------------------------------------------------------------

def uninstall(user):
    """
    Remove NovaShell files.

    Only files belonging to NovaShell are removed.
    """
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

    systemctl = command_path(
        "systemctl"
    )

    # For a user installation, stop the user's service.
    #
    # For a system installation invoked through sudo, don't run
    # systemctl --user as root.
    if systemctl is not None and user:
        run(
            [
                str(systemctl),
                "--user",
                "disable",
                "--now",
                "novashell.service",
            ]
        )

        run(
            [
                str(systemctl),
                "--user",
                "daemon-reload",
            ]
        )

    print()
    log("Removing NovaShell")

    removed_any = False

    for path in files:
        if not path.exists():
            continue

        try:
            path.unlink()

            print(
                "    removed {}".format(
                    path
                )
            )

            removed_any = True

        except OSError as exc:
            warn(
                "Could not remove {}: {}".format(
                    path,
                    exc,
                )
            )

    if not user:
        print()
        info(
            "System service file removed."
        )

        info(
            "If NovaShell was previously enabled for a user,"
        )

        info(
            "disable it from that user's session with:"
        )

        info(
            "systemctl --user disable --now novashell.service"
        )

    if not removed_any:
        info(
            "No NovaShell installation files were found."
        )


# ---------------------------------------------------------------------------
# Environment check
# ---------------------------------------------------------------------------

def check_runtime_helpers():
    print()
    info("Optional runtime helpers:")

    for executable, description in RUNTIME_HINTS:
        status = (
            "OK "
            if command_exists(executable)
            else "---"
        )

        print(
            "    {} {:18s} {}".format(
                status,
                executable,
                description,
            )
        )


def check_rust_read_only():
    """
    Read-only Rust check.
    """
    ensure_cargo_path()

    version = rustc_version()

    if version is None:
        error(
            "rustc not found."
        )

        print(
            "    Run normal setup to install Rust."
        )

        return False

    text = rust_version_string()

    info(
        "rustc: {}".format(
            text
        )
    )

    if not rust_version_satisfies(
        version
    ):
        error(
            "Rust >= {}.{} is required.".format(
                MIN_RUST_MAJOR,
                MIN_RUST_MINOR,
            )
        )

        return False

    return True


def check_mode():
    """
    Inspect only.

    This function MUST NOT:
        - apt update
        - apt install
        - rustup update
        - write files
        - build anything
        - modify configuration
    """
    log(
        "NovaShell environment check"
    )

    print()
    print(
        "    No changes will be made."
    )

    print()
    print_os_info()

    print()
    info(
        "Python: {}.{}.{}".format(
            sys.version_info.major,
            sys.version_info.minor,
            sys.version_info.micro,
        )
    )

    print()
    info(
        "Project: {}".format(
            project_root()
        )
    )

    try:
        validate_project_files()
        info(
            "Project files: OK"
        )
    except SystemExit:
        return 1

    print()
    info("Rust:")

    rust_ok = check_rust_read_only()

    print()
    info("Build libraries:")

    try:
        check_dev_headers()
        libraries_ok = True
    except SystemExit:
        libraries_ok = False

    check_cargo_dependency_configuration()

    check_runtime_helpers()

    print()
    info("Summary:")

    if rust_ok and libraries_ok:
        print(
            "    Environment looks ready for building."
        )
        return 0

    print(
        "    Environment is not ready for building."
    )

    return 1


# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------

def main():
    ensure_python_version()

    parser = argparse.ArgumentParser(
        description=(
            "Bootstrap, build, install, "
            "check, or uninstall NovaShell."
        )
    )

    parser.add_argument(
        "--user",
        action="store_true",
        help=(
            "Install/remove NovaShell for the current "
            "user instead of system-wide."
        ),
    )

    parser.add_argument(
        "--check",
        action="store_true",
        help=(
            "Inspect the environment only; "
            "do not install, update, build, or modify anything."
        ),
    )

    parser.add_argument(
        "--uninstall",
        action="store_true",
        help="Remove NovaShell installation files.",
    )

    args = parser.parse_args()

    ensure_linux()

    # --check must happen before any operation that changes
    # the machine.
    if args.check:
        return check_mode()

    # Validate the repository before installing dependencies.
    validate_project_files()

    if args.uninstall:
        uninstall(
            user=args.user
        )

        log(
            "NovaShell uninstall complete."
        )

        return 0

    print()
    log(
        "NovaShell setup ({})".format(
            "user" if args.user else "system"
        )
    )

    print()
    print_os_info()

    # ------------------------------------------------------------------
    # 1. Make sure Rust exists.
    # ------------------------------------------------------------------

    ensure_rust()

    # ------------------------------------------------------------------
    # 2. Install native build dependencies.
    # ------------------------------------------------------------------

    ensure_system_deps()

    # ------------------------------------------------------------------
    # 3. Validate native libraries BEFORE Cargo compilation.
    # ------------------------------------------------------------------

    check_dev_headers()

    # ------------------------------------------------------------------
    # 4. Check the Cargo dependency configuration.
    # ------------------------------------------------------------------

    check_cargo_dependency_configuration()

    # ------------------------------------------------------------------
    # 5. Show dependency information.
    # ------------------------------------------------------------------

    ensure_cargo_available()

    cargo_tree_summary()

    # ------------------------------------------------------------------
    # 6. Build.
    # ------------------------------------------------------------------

    cargo_build()

    # ------------------------------------------------------------------
    # 7. Verify resulting binary.
    # ------------------------------------------------------------------

    binary = verify_binary()

    # ------------------------------------------------------------------
    # 8. Install.
    # ------------------------------------------------------------------

    install_files(
        user=args.user
    )

    # ------------------------------------------------------------------
    # 9. Final information.
    # ------------------------------------------------------------------

    print()

    log(
        "NovaShell installation complete."
    )

    print()

    info(
        "Binary:"
    )

    print(
        "        {}".format(
            binary
        )
    )

    print()

    info(
        "Test:"
    )

    print(
        "        novashell --windowed"
    )

    print()

    info(
        "Start user service:"
    )

    print(
        "        systemctl --user daemon-reload"
    )

    print(
        "        systemctl --user enable --now novashell.service"
    )

    print()

    info(
        "Uninstall:"
    )

    if args.user:
        print(
            "        python3 setup.py --user --uninstall"
        )
    else:
        print(
            "        sudo python3 setup.py --uninstall"
        )

    print()

    info(
        "No GRUB, kernel, initramfs, fstab, "
        "display-manager, or GNOME configuration was modified."
    )

    return 0


if __name__ == "__main__":
    try:
        sys.exit(
            main()
        )

    except KeyboardInterrupt:
        print()
        error(
            "Interrupted by user."
        )
        sys.exit(130)

    except BrokenPipeError:
        # Allows things like:
        # python3 setup.py --check | head
        sys.exit(0)

    except Exception as exc:
        print()
        error(
            "Unexpected error: {}".format(
                exc
            )
        )

        print()
        print(
            "    Python: {}.{}.{}".format(
                sys.version_info.major,
                sys.version_info.minor,
                sys.version_info.micro,
            )
        )

        print(
            "    Project: {}".format(
                project_root()
            )
        )

        sys.exit(1)