// SPDX-License-Identifier: GPL-3.0-or-later
//! wlRIX power status: the battery, as an icon in `wlrix-tray`, and low-battery warnings.

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut rest = args.iter();
    if let Some(argument) = rest.next() {
        match argument.as_str() {
            "--help" | "-h" => {
                println!(
                    "wlrix-power {}\n\n\
                     Power status for the wlRIX desktop: the battery as a StatusNotifierItem,\n\
                     drawn by wlrix-tray, low-battery warnings, and power modes. Started by\n\
                     wlrix-session; needs UPower on the system bus and a session D-Bus.\n\n\
                     Usage: wlrix-power [options]\n\n\
                     Options:\n  \
                       --status               read UPower once, say what the icon\n                         \
                                              would show, exit\n  \
                       --check-config <path>  say whether that file would be accepted, exit\n  \
                       -h, --help             this message\n  \
                       -V, --version          print the version\n\n\
                     Settings live in ~/.config/wlrix/power.toml, or /etc/wlrix/power.toml.",
                    env!("CARGO_PKG_VERSION")
                );
                return;
            }
            "--version" | "-V" => {
                println!("wlrix-power {}", env!("CARGO_PKG_VERSION"));
                return;
            }
            "--check-config" => {
                let Some(path) = rest.next() else {
                    eprintln!("wlrix-power: --check-config needs a path");
                    std::process::exit(2);
                };
                if let Err(why) = wlrix_power::config::check(std::path::Path::new(path)) {
                    eprintln!("{why}");
                    std::process::exit(1);
                }
                return;
            }
            "--status" => {
                if let Err(err) = wlrix_power::daemon::status() {
                    eprintln!("wlrix-power: {err}");
                    std::process::exit(1);
                }
                return;
            }
            other => {
                eprintln!("wlrix-power: unknown argument: {other}");
                std::process::exit(2);
            }
        }
    }

    if let Err(err) = wlrix_power::daemon::run() {
        eprintln!("wlrix-power: {err}");
        std::process::exit(1);
    }
}
