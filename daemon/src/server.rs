//! Unix-socket front door. One JSON array per line: either
//! `["listen", topic]` (stream) or a command (`["toggle","launcher"]`).

use anyhow::{Result, anyhow, bail};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};

use crate::app::app;

pub fn bind() -> Result<UnixListener> {
    let path = crate::util::socket_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    // Refuse to start twice for the same Wayland display.
    if std::os::unix::net::UnixStream::connect(&path).is_ok() {
        bail!("another dmr daemon is already running ({})", path.display());
    }
    let _ = std::fs::remove_file(&path);
    let l = UnixListener::bind(&path)?;
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    Ok(l)
}

pub async fn serve(l: UnixListener) {
    loop {
        match l.accept().await {
            Ok((s, _)) => {
                tokio::spawn(handle(s));
            }
            Err(e) => {
                eprintln!("dmr: accept: {e}");
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }
        }
    }
}

async fn handle(s: UnixStream) {
    let (r, mut w) = s.into_split();
    let mut r = BufReader::new(r);
    let mut line = String::new();
    if r.read_line(&mut line).await.unwrap_or(0) == 0 {
        return;
    }
    let args: Vec<String> = match serde_json::from_str::<Vec<serde_json::Value>>(line.trim()) {
        Ok(v) => v.into_iter().map(|x| x.as_str().map(str::to_string).unwrap_or_else(|| x.to_string())).collect(),
        Err(_) => line.split_whitespace().map(str::to_string).collect(),
    };
    if args.first().map(String::as_str) == Some("listen") {
        let topic = args.get(1).cloned().unwrap_or_default();
        let Some(mut rx) = app().hub.subscribe(&topic) else {
            let _ = w.write_all(b"{}\n").await;
            return;
        };
        loop {
            let v = rx.borrow_and_update().clone();
            // Never push the placeholder: EWW keeps its :initial until real data.
            if &*v == "{}" {
                if rx.changed().await.is_err() {
                    return;
                }
                continue;
            }
            let mut buf = Vec::with_capacity(v.len() + 1);
            buf.extend_from_slice(v.as_bytes());
            buf.push(b'\n');
            if w.write_all(&buf).await.is_err() {
                return;
            }
            if rx.changed().await.is_err() {
                return;
            }
        }
    }
    let reply = match dispatch(args).await {
        Ok(s) if s.is_empty() => "ok".to_string(),
        Ok(s) => s,
        Err(e) => format!("ERR {e}"),
    };
    // One reply per line: multi-line output travels with record separators.
    let reply = reply.replace('\n', "\u{1e}");
    let _ = w.write_all(reply.as_bytes()).await;
    let _ = w.write_all(b"\n").await;
}

fn arg(a: &[String], i: usize) -> &str {
    a.get(i).map(String::as_str).unwrap_or("")
}

fn pct_arg(s: &str) -> Result<f64> {
    let v: f64 = s.trim_end_matches('%').parse().map_err(|_| anyhow!("expected a number, got {s:?}"))?;
    Ok(crate::util::clamp(v, 0.0, 100.0) / 100.0)
}

pub async fn dispatch(a: Vec<String>) -> Result<String> {
    let app = app();
    let cmd = arg(&a, 0);
    let sub = arg(&a, 1);
    match cmd {
        "get" => Ok(app.hub.get(sub).map(|s| s.to_string()).unwrap_or_default()),
        "ping" => Ok("pong".into()),
        "toggle" | "open" if matches!(sub, "dash" | "dashboard") && !arg(&a, 2).is_empty() => {
            // DMS `dash toggle <tab>`: switch tab; toggle only when already on it.
            let tab = match arg(&a, 2) {
                "media" | "1" => 1,
                "wallpaper" | "wallpapers" | "2" => 2,
                "weather" | "3" => 3,
                _ => 0,
            };
            let same = app.hub.get("ui").and_then(|u| serde_json::from_str::<serde_json::Value>(&u).ok()).map(|u| u["dashTab"] == tab).unwrap_or(false);
            app.ui.set_dash_tab(tab);
            if cmd == "toggle" && same { app.ui.toggle("dash").await } else { app.ui.open("dash").await }
        }
        "toggle" => app.ui.toggle(sub).await,
        "open" => app.ui.open(sub).await,
        "close" => {
            if sub.is_empty() || sub == "all" {
                app.ui.close_all().await;
                Ok(String::new())
            } else {
                app.ui.close(sub).await
            }
        }
        "hover" => {
            app.ui.hover(sub, arg(&a, 2)).await;
            Ok(String::new())
        }
        "unhover" => {
            app.ui.unhover(sub).await;
            Ok(String::new())
        }
        "dash-tab" => {
            app.ui.set_dash_tab(sub.parse().unwrap_or(0));
            Ok(String::new())
        }
        "dash-tab-scroll" => {
            app.ui.dash_tab_step(if sub == "down" { 1 } else { -1 });
            Ok(String::new())
        }
        "cc-page" => {
            app.ui.set_misc("ccPage", serde_json::Value::from(sub));
            app.net.set_list_demand(sub == "network");
            if sub == "network" {
                let _ = app.net.command("scan", &[]).await;
            }
            Ok(String::new())
        }
        "power" => match sub {
            "key" => app.ui.power_key(arg(&a, 2)).await,
            "" => app.ui.toggle("powermenu").await,
            act => app.ui.power_action(act).await,
        },
        "night" => {
            toggle_night();
            Ok(String::new())
        }
        "volume" => app.audio.command(false, sub, arg(&a, 2)).await,
        "mic" => app.audio.command(true, sub, arg(&a, 2)).await,
        "audio" => match sub {
            "sink" => app.audio.set_default(false, arg(&a, 2)).await,
            "source" => app.audio.set_default(true, arg(&a, 2)).await,
            _ => bail!("audio sink|source <name>"),
        },
        "brightness" => {
            match sub {
                "up" => app.bright.step(1.0).await,
                "down" => app.bright.step(-1.0).await,
                "set" => app.bright.set(pct_arg(arg(&a, 2))?).await,
                _ => bail!("brightness up|down|set <pct>"),
            }
            Ok(String::new())
        }
        "media" => app.media.command(sub, arg(&a, 2)).await,
        "notifs" => app.notifs.command(sub, &a[2.min(a.len())..]).await,
        "wallpaper" => app.theme.wallpaper_cmd(sub, arg(&a, 2)).await,
        "scheme" => app.theme.scheme_cmd(&a[1.min(a.len())..]).await,
        "wifi" => app.net.command(sub, &a[2.min(a.len())..]).await,
        "bluetooth" | "bt" => app.bt.command(sub, arg(&a, 2)).await,
        "launcher" => app.launcher.command(sub, &a[2.min(a.len())..]).await,
        "config" => {
            let rest: Vec<&str> = a.iter().skip(3).map(String::as_str).collect();
            crate::settings::command(sub, arg(&a, 2), &rest).await
        }
        "settings" => {
            // Open the settings file in the user's editor (DMS opens its
            // Settings window here; `dmr config set` changes keys live).
            app.ui.close_all().await;
            let path = crate::util::config_dir().join("settings.json");
            if !path.exists() {
                let _ = std::fs::create_dir_all(crate::util::config_dir());
                let _ = std::fs::write(&path, "{\n}\n");
            }
            let term = crate::app::cfg().general.terminal.clone();
            let q = crate::util::shell_quote(&path.to_string_lossy());
            crate::util::spawn_sh(&format!("if [ -n \"$EDITOR\" ]; then {term} -e \"$EDITOR\" {q}; else xdg-open {q}; fi"));
            Ok(String::new())
        }
        "session" => {
            app.ui.close_all().await;
            session(sub).await
        }
        "lock" => {
            app.ui.close_all().await;
            lock();
            // Give the locker time to grab the session (swayidle before-sleep
            // waits for this command).
            tokio::time::sleep(std::time::Duration::from_millis(800)).await;
            Ok(String::new())
        }
        "idle-inhibit" => {
            toggle_idle_inhibit();
            Ok(String::new())
        }
        "gamemode" => {
            crate::theme::toggle_gamemode().await;
            Ok(String::new())
        }
        "dnd" => {
            let on = {
                let mut m = app.misc.lock().unwrap();
                m.dnd = !m.dnd;
                m.dnd
            };
            // Shown even with DND on (toasts are shell feedback, not notifications).
            app.notifs.toast(
                crate::app::cfg().utilities.toasts.dnd_changed,
                if on { "Do not disturb enabled" } else { "Do not disturb disabled" },
                if on { "Notifications are silenced" } else { "Notifications will pop up again" },
                if on { "notifications_off" } else { "notifications_active" },
            );
            app.publish_status();
            Ok(String::new())
        }
        "power-profile" => {
            if !sub.is_empty() {
                crate::util::run_ok(&["powerprofilesctl", "set", sub], 3000).await;
            }
            app.bat.refresh_profile().await;
            Ok(String::new())
        }
        "exec" => {
            let rest = a[1..].join(" ");
            app.ui.close_all().await;
            crate::util::spawn_sh(&rest);
            Ok(String::new())
        }
        "app" => {
            // Launch configured default apps (terminal/audio/explorer).
            let cmdline = {
                let c = crate::app::cfg();
                match sub {
                    "terminal" => c.general.terminal.clone(),
                    "audio" => c.general.audio.clone(),
                    "explorer" => c.general.explorer.clone(),
                    _ => bail!("app terminal|audio|explorer"),
                }
            };
            app.ui.close_all().await;
            crate::util::spawn_sh(&cmdline);
            Ok(String::new())
        }
        "screenshot" => {
            app.ui.close_all().await;
            crate::theme::screenshot(sub).await
        }
        "lockinfo" => Ok(lockinfo(sub)),
        "cal" => {
            app.info.cal_cmd(sub);
            Ok(String::new())
        }
        "ws" | "workspace" if sub == "idx" => app.wm.goto(&format!("idx {}", arg(&a, 2))).await,
        "ws" | "workspace" => app.wm.goto(sub).await,
        "ws-scroll" => app.wm.scroll(sub).await,
        "window" => app.wm.window_cmd(sub, arg(&a, 2)).await,
        "reload" => {
            *app.cfg.write().unwrap() = std::sync::Arc::new(crate::config::Config::load());
            crate::settings::publish();
            app.wm.republish();
            app.publish_status();
            app.theme.apply_current().await;
            app.eww.reload().await;
            Ok(String::new())
        }
        "quit" => {
            app.ui.request_quit();
            Ok(String::new())
        }
        _ => bail!("unknown command {cmd:?} (see `dmr --help`)"),
    }
}

pub fn lock() {
    let c = crate::app::cfg();
    let argv: Vec<&str> = c.lock.command.iter().map(String::as_str).collect();
    // Never stack two lockers.
    if crate::util::which("pidof") {
        let name = argv.first().copied().unwrap_or("hyprlock");
        if std::process::Command::new("pidof").arg(name).output().map(|o| o.status.success()).unwrap_or(false) {
            return;
        }
    }
    crate::util::spawn_argv(&argv);
}

pub async fn session(what: &str) -> Result<String> {
    let c = crate::app::cfg().session.clone();
    let argv: Vec<String> = match what {
        "logout" => c.logout,
        "shutdown" | "poweroff" => c.shutdown,
        "reboot" => c.reboot,
        "hibernate" => c.hibernate,
        "suspend" | "sleep" => vec!["systemctl".into(), "suspend-then-hibernate".into()],
        "lock" => {
            lock();
            return Ok(String::new());
        }
        _ => bail!("session logout|shutdown|reboot|hibernate|suspend|lock"),
    };
    if what == "suspend" || what == "sleep" {
        // Caelestia "Sleep": suspend then hibernate; fall back to plain suspend.
        if !crate::util::run_ok(&["systemctl", "suspend-then-hibernate"], 5000).await {
            crate::util::spawn_argv(&["systemctl", "suspend"]);
        }
        return Ok(String::new());
    }
    let argv: Vec<&str> = argv.iter().map(String::as_str).collect();
    crate::util::spawn_argv(&argv);
    Ok(String::new())
}

/// Night mode (DMS "night light"): a gamma tint through wlsunset, which
/// works on any wlr-gamma-control compositor.
fn toggle_night() {
    let app = app();
    {
        let mut m = app.misc.lock().unwrap();
        if let Some(mut c) = m.night_child.take() {
            let _ = c.kill();
            let _ = c.wait();
            m.night = false;
        } else {
            let t = crate::app::cfg().general.night_temp.to_string();
            let child = std::process::Command::new("wlsunset")
                .args(["-T", &(t.parse::<u32>().unwrap_or(4500) + 1).to_string(), "-t", &t, "-S", "00:00", "-s", "23:59"])
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn();
            if let Ok(c) = child {
                m.night_child = Some(c);
                m.night = true;
            }
        }
    }
    app.publish_status();
}

fn toggle_idle_inhibit() {
    let app = app();
    {
        let mut m = app.misc.lock().unwrap();
        if let Some(mut c) = m.idle_inhibitor.take() {
            let _ = c.kill();
            let _ = c.wait();
            m.idle_inhibit = false;
        } else {
            // systemd-inhibit blocks idle (swayidle/hypridle honour logind
            // inhibitors) for as long as the child lives.
            let child = std::process::Command::new("systemd-inhibit")
                .args(["--what=idle", "--who=dmr", "--why=Keep awake", "--mode=block", "sleep", "infinity"])
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn();
            if let Ok(c) = child {
                m.idle_inhibitor = Some(c);
                m.idle_inhibit = true;
            }
        }
    }
    app.publish_status();
}

/// Text snippets for the hyprlock lock screen (templates/hyprlock.conf).
fn lockinfo(key: &str) -> String {
    let app = app();
    let get = |t: &str| app.hub.get(t).and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok()).unwrap_or_default();
    let s = |v: &serde_json::Value| v.as_str().map(str::to_string).unwrap_or_else(|| v.to_string());
    let esc = |t: &str| t.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;");
    match key {
        "weather-big" => {
            let w = get("info")["weather"].clone();
            let (t, i) = if w["available"] == true { (s(&w["temp"]), s(&w["icon"])) } else { ("--°C".into(), "cloud".into()) };
            format!("{}  <span font_desc=\"Material Symbols Rounded\">{i}</span>", esc(&t))
        }
        "media-title" => {
            let m = get("media");
            esc(&if m["available"] == true { s(&m["title"]) } else { String::new() })
        }
        "media-artist" => {
            let m = get("media");
            esc(&if m["available"] == true { s(&m["artist"]) } else { String::new() })
        }
        _ => String::new(),
    }
}
