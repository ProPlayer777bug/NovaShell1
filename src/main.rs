#[cfg(target_os = "linux")]
fn main() {
    let opts = novashell::shell::parse_args();
    let result = if opts.watchdog {
        novashell::shell::watchdog(&opts)
    } else {
        novashell::shell::run(opts.clone())
    };
    if let Err(e) = result {
        eprintln!("novashell: {e:#}");
        std::process::exit(1);
    }
}

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("NovaShell requires Linux (GTK4 + WebKitGTK6) to run its shell.");
    eprintln!("On any platform the logic modules are tested with:  cargo test");
    std::process::exit(1);
}