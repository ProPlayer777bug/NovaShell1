//! NovaShell — an original console-style desktop shell for Ubuntu.
//!
//! Architecture:
//!   - Compiled Rust core shell (GTK4 + WebKitGTK6).
//!   - Offline-capable HTML/CSS/JS UI rendered inside a WebKitWebView.
//!   - gilrs (evdev) for controller input, mapped to abstract actions.
//!   - Plugin-style launcher providers (Steam / Heroic / desktop entries).
//!
//! Logic modules (config, library, launchers, exec parsing…) are kept
//! platform-agnostic so they can be unit-tested with `cargo test` anywhere.
//! GUI-only modules are gated with `#[cfg(target_os = "linux")]`.

#[cfg(target_os = "linux")]
pub mod controllers;
pub mod games;
pub mod integrations;
pub mod launcher;
pub mod plugins;
pub mod settings;
#[cfg(target_os = "linux")]
pub mod system;

#[cfg(target_os = "linux")]
pub mod shell;
#[cfg(target_os = "linux")]
pub mod ui;

pub mod util;

pub use settings::Config;