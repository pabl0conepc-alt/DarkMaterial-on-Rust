//! dmr — a DankMaterialShell-style desktop shell for driftwm, rendered by EWW.
//!
//! One binary, three roles:
//!   dmr daemon          the event-driven backend (one process)
//!   dmr listen <topic>  stream a topic as JSON lines (used by EWW deflisten)
//!   dmr <command...>    send a command to the daemon

mod app;
mod audio;
mod battery;
mod bluetooth;
mod brightness;
mod calc;
mod client;
mod clock;
mod config;
mod eww;
mod hub;
mod info;
mod launcher;
mod media;
mod network;
mod notifd;
mod server;
mod settings;
mod setup;
mod dbus;
mod sysmon;
mod theme;
mod ui;
mod util;
mod wm;

const HELP: &str = "dmr — DankMaterialShell-style shell for driftwm + EWW

USAGE:
  dmr setup [--check|--force|--no-terminal]  per-user setup (driftwm + terminal configs, wallpapers) + health check
  dmr daemon                     start the shell (backend + EWW)
  dmr listen <topic>             stream topic JSON (wm status media notifs sys launcher ui theme info)
  dmr get <topic>                print a topic once
  dmr toggle <panel>             launcher | dashboard | session | sidebar | utilities
  dmr open|close <panel>         (close with no panel closes everything)
  dmr volume up|down|mute|set <0-100>   dmr mic mute|up|down|set <0-100>
  dmr brightness up|down|set <0-100>
  dmr media play-pause|next|previous|stop|seek <s>|player <name>
  dmr notifs clear|dnd|dismiss <id>|action <id> <key>
  dmr wallpaper set <file>|random|list
  dmr scheme set <name> [flavour] [mode] | mode dark|light|toggle | variant <name> | list
  dmr wifi toggle|scan|connect <ssid> [password]|disconnect
  dmr bluetooth toggle|scan|connect <mac>|disconnect <mac>
  dmr lock | session <logout|shutdown|reboot|hibernate|suspend>
  dmr idle-inhibit toggle | gamemode toggle
  dmr nexus open [page]          settings panel        dmr config get|set <key> [value]
  dmr reload                     reload config + EWW
  dmr quit
";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let code = match args.first().map(String::as_str) {
        None | Some("-h" | "--help" | "help") => {
            print!("{HELP}");
            0
        }
        Some("-V" | "--version") => {
            println!("dmr {}", env!("CARGO_PKG_VERSION"));
            0
        }
        Some("daemon") => match app::run_daemon() {
            Ok(()) => 0,
            Err(e) => {
                eprintln!("dmr: {e:#}");
                1
            }
        },
        Some("setup") => setup::run(&args[1..]),
        Some("listen") => client::listen(args.get(1).map(String::as_str).unwrap_or("")),
        _ => client::send(&args),
    };
    std::process::exit(code);
}
