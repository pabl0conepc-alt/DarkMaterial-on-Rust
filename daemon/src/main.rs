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

const HELP: &str = "dmr — DankMaterialShell recreated for driftwm (EWW + Rust)

USAGE:
  dmr setup [--check|--force]       per-user setup (driftwm config with DMS keybinds, wallpapers) + health check
  dmr daemon                        start the shell (backend + EWW)
  dmr toggle|open|close <panel>     launcher (spotlight) | controlcenter | dash | notifcenter | powermenu | osd
  dmr toggle dash <tab>             overview | media | wallpaper | weather (DMS `dash toggle <tab>`)
  dmr close                         close every panel
  dmr volume up|down|mute|set <0-100>     dmr mic mute|up|down|set <0-100>
  dmr brightness up|down|set <0-100>
  dmr media play-pause|next|previous|stop|seek-to <s|N%>
  dmr notifs clear|dismiss <id>|action <id> <n>     dmr dnd     dmr night     dmr idle-inhibit
  dmr power [reboot|logout|poweroff|lock|suspend|restart]
  dmr wallpaper set <file>|random|list      dmr scheme set dynamic|dms purple|catppuccin mocha|…
  dmr scheme mode dark|light|toggle         dmr scheme variant tonalspot|vibrant|…
  dmr wifi toggle|scan|disconnect           dmr bluetooth toggle|scan|connect <mac>
  dmr lock | session logout|shutdown|reboot|hibernate|suspend
  dmr config get|set <key> [value]          e.g. dmr config set bar.floating false
  dmr settings                      open ~/.config/dmr/settings.json in $EDITOR
  dmr get <topic> | listen <topic>  wm status media notifs load sys launcher ui theme info clock
  dmr reload | quit
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
