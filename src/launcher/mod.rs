//! Game / application launching helpers.
//!
//! The shell never blocks on a launched game: each launch is spawned into its
//! own process group and optionally monitored on a background thread so the
//! UI can react when the title exits.

use anyhow::{anyhow, Result};
use std::path::PathBuf;
use std::process::{Child, Command};

/// Describes anything NovaShell can launch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Spec {
    /// Display name (used in logs and the "now playing" state).
    pub name: String,
    /// Executable path or command name.
    pub program: String,
    pub args: Vec<String>,
    pub cwd: Option<PathBuf>,
    pub env: Vec<(String, String)>,
}

impl Spec {
    pub fn new(name: impl Into<String>, program: impl Into<String>) -> Self {
        Spec {
            name: name.into(),
            program: program.into(),
            args: vec![],
            cwd: None,
            env: vec![],
        }
    }

    /// Append one argument.
    pub fn arg(mut self, a: impl Into<String>) -> Self {
        self.args.push(a.into());
        self
    }

    /// Append several arguments, keeping anything already added.
    ///
    /// This appends rather than replaces: runtimes build a spec by adding the
    /// program they need first (`wine <exe>`, `python3 <proton> run <exe>`,
    /// `pcsx2 <rom>`) and then the caller's arguments, so replacing here would
    /// silently drop the program argument. Use [`Spec::set_args`] to replace.
    pub fn args(mut self, args: impl IntoIterator<Item = String>) -> Self {
        self.args.extend(args);
        self
    }

    /// Replace the argument list outright.
    pub fn set_args(mut self, args: Vec<String>) -> Self {
        self.args = args;
        self
    }

    pub fn cwd(mut self, cwd: PathBuf) -> Self {
        self.cwd = Some(cwd);
        self
    }

    pub fn env(mut self, k: impl Into<String>, v: impl Into<String>) -> Self {
        self.env.push((k.into(), v.into()));
        self
    }
}

/// Spawn the process in its own process group so the shell's death can never
/// take the game with it, and Ctrl+C in dev mode doesn't kill the game either.
#[cfg(target_os = "linux")]
pub fn spawn(spec: &Spec) -> Result<Child> {
    use std::os::unix::process::CommandExt;

    let mut cmd = Command::new(&spec.program);
    cmd.args(&spec.args);
    if let Some(cwd) = &spec.cwd {
        cmd.current_dir(cwd);
    }
    for (k, v) in &spec.env {
        cmd.env(k, v);
    }
    // New process group (pgid 0 => the child's own new group). Safe on Linux.
    cmd.process_group(0);
    cmd.stdin(std::process::Stdio::null());
    cmd.spawn()
        .map_err(|e| anyhow!("failed to launch `{}`: {e}", spec.program))
}

#[cfg(not(target_os = "linux"))]
pub fn spawn(spec: &Spec) -> Result<Child> {
    Command::new(&spec.program)
        .args(&spec.args)
        .current_dir(spec.cwd.clone().unwrap_or_default())
        .spawn()
        .map_err(|e| anyhow!("failed to launch `{}`: {e}", spec.program))
}

/// Run a command and capture its stdout (used for small system queries).
pub fn run_capture(program: &str, args: &[&str]) -> Result<String> {
    let out = Command::new(program)
        .args(args)
        .output()
        .map_err(|e| anyhow!("failed to run `{program}`: {e}"))?;
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

/// Parse a `.desktop` `Exec=` line into (program, args).
///
/// Handles double/backslash quoting, `%` field codes, and environment
/// assignments such as `env VAR=value program`.
pub fn parse_desktop_exec(exec: &str) -> Option<(String, Vec<String>)> {
    let tokens = tokenize_exec(exec)?;
    if tokens.is_empty() {
        return None;
    }

    let mut tokens = tokens;
    // Strip a leading "env VAR=val ..." preamble.
    if tokens.first().map(|t| t == "env").unwrap_or(false) {
        let mut i = 1;
        while i < tokens.len() && {
            let t = &tokens[i];
            t.contains('=') && !t.starts_with('-')
        } {
            i += 1;
        }
        tokens.drain(..i.min(tokens.len()));
    }
    if tokens.is_empty() {
        return None;
    }
    let program = tokens.remove(0);
    // Strip field codes (%f %u %F %U %c %i %k) that tools substitute.
    let args: Vec<String> = tokens
        .into_iter()
        .filter(|t| !t.starts_with('%'))
        .collect();
    Some((program, args))
}

fn tokenize_exec(exec: &str) -> Option<Vec<String>> {
    let mut tokens = vec![];
    let mut cur = String::new();
    let mut chars = exec.chars().peekable();
    let mut in_tok = false;

    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                if let Some(next) = chars.next() {
                    cur.push(next);
                    in_tok = true;
                }
            }
            '"' => {
                // Consume until closing quote, preserving content.
                let mut lit = String::new();
                for c2 in chars.by_ref() {
                    if c2 == '"' {
                        break;
                    }
                    lit.push(c2);
                }
                if !lit.is_empty() {
                    cur.push_str(&lit);
                    in_tok = true;
                }
            }
            c if c.is_whitespace() => {
                if in_tok {
                    tokens.push(std::mem::take(&mut cur));
                    in_tok = false;
                }
            }
            c => {
                cur.push(c);
                in_tok = true;
            }
        }
    }
    if in_tok {
        tokens.push(cur);
    }
    Some(tokens)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn args_appends_and_set_args_replaces() {
        let s = Spec::new("t", "prog").arg("first").args(vec!["second".into(), "third".into()]);
        assert_eq!(s.args, vec!["first", "second", "third"]);
        let replaced = s.set_args(vec!["only".into()]);
        assert_eq!(replaced.args, vec!["only"]);
    }

    #[test]
    fn parses_simple_exec() {
        let (p, a) = parse_desktop_exec("/usr/bin/steam -applaunch 570").unwrap();
        assert_eq!(p, "/usr/bin/steam");
        assert_eq!(a, vec!["-applaunch", "570"]);
    }

    #[test]
    fn strips_field_codes() {
        let (p, a) = parse_desktop_exec("firefox-esr %u --new-window %U").unwrap();
        assert_eq!(p, "firefox-esr");
        assert_eq!(a, vec!["--new-window"]);
    }

    #[test]
    fn handles_quoted_args_with_spaces() {
        let (p, a) = parse_desktop_exec("\"My Program\" \"dir with space/a.exe\" -x").unwrap();
        assert_eq!(p, "My Program");
        assert_eq!(a, vec!["dir with space/a.exe", "-x"]);
    }

    #[test]
    fn strips_env_prefix() {
        let (p, a) = parse_desktop_exec("env GDK_BACKEND=wayland mygame --fullscreen").unwrap();
        assert_eq!(p, "mygame");
        assert_eq!(a, vec!["--fullscreen"]);
    }

    #[test]
    fn empty_returns_none() {
        assert!(parse_desktop_exec("").is_none());
    }
}