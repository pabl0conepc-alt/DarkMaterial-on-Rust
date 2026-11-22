//! Owns the EWW instance: builds a runtime config dir (symlinks to the shipped
//! yuck/scss + generated theme), starts `eww daemon`, opens/closes windows.

use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

/// Signalled by the watchdog; `restart_on_crash` (spawned once) rebuilds the UI.
static CRASHED: tokio::sync::Notify = tokio::sync::Notify::const_new();

pub async fn restart_on_crash() {
    loop {
        CRASHED.notified().await;
        crate::app::app().ui.restart_shell().await;
    }
}

pub struct Eww {
    pub dir: PathBuf,
    bin: String,
    /// PID of the running `eww daemon` (owned by its watchdog task).
    pid: Mutex<Option<u32>>,
    /// Bumped on every start/kill: a watchdog only reacts to its own daemon.
    generation: AtomicU64,
    /// Recent unexpected exits, to stop restarting a daemon that keeps dying.
    crashes: Mutex<Vec<std::time::Instant>>,
}

fn real_config_home() -> PathBuf {
    match std::env::var_os("XDG_CONFIG_HOME") {
        Some(v) if !v.is_empty() => PathBuf::from(v),
        _ => crate::util::home().join(".config"),
    }
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// fontconfig for EWW: system config, the user's own config (Home Manager
/// fonts live there) and the package's font directories (`DMR_FONT_DIRS`).
fn fontconfig() -> String {
    let user = real_config_home().join("fontconfig");
    let mut s = String::from("<?xml version=\"1.0\"?>\n<!DOCTYPE fontconfig SYSTEM \"urn:fontconfig:fonts.dtd\">\n<fontconfig>\n");
    // An explicit FONTCONFIG_FILE from the session wins over the system file.
    let base = std::env::var("FONTCONFIG_FILE").ok().filter(|f| !f.is_empty() && !f.ends_with("dmr-fonts.conf"));
    s.push_str(&format!(
        "  <include ignore_missing=\"yes\">{}</include>\n",
        xml_escape(base.as_deref().unwrap_or("/etc/fonts/fonts.conf"))
    ));
    for p in [user.join("fonts.conf"), user.join("conf.d")] {
        s.push_str(&format!("  <include ignore_missing=\"yes\">{}</include>\n", xml_escape(&p.to_string_lossy())));
    }
    for d in [crate::util::home().join(".local/share/fonts"), crate::util::home().join(".nix-profile/share/fonts")] {
        s.push_str(&format!("  <dir>{}</dir>\n", xml_escape(&d.to_string_lossy())));
    }
    if let Some(dirs) = std::env::var_os("DMR_FONT_DIRS") {
        for d in std::env::split_paths(&dirs) {
            s.push_str(&format!("  <dir>{}</dir>\n", xml_escape(&d.to_string_lossy())));
        }
    }
    s.push_str("</fontconfig>\n");
    s
}

/// The user's GTK icon theme, else the first well-known one installed.
fn icon_theme() -> String {
    let ini = real_config_home().join("gtk-3.0/settings.ini");
    if let Ok(s) = std::fs::read_to_string(ini)
        && let Some(v) = s.lines().find_map(|l| l.trim().strip_prefix("gtk-icon-theme-name").map(|r| r.trim_start_matches([' ', '=']).trim().to_string()))
        && !v.is_empty() {
            return v;
        }
    let data_dirs = std::env::var("XDG_DATA_DIRS").unwrap_or_else(|_| "/usr/share:/run/current-system/sw/share".into());
    let mut roots: Vec<PathBuf> = vec![crate::util::home().join(".local/share/icons"), crate::util::home().join(".icons")];
    roots.extend(data_dirs.split(':').filter(|d| !d.is_empty()).map(|d| PathBuf::from(d).join("icons")));
    for theme in ["Papirus-Dark", "Papirus", "breeze-dark", "breeze", "Adwaita"] {
        if roots.iter().any(|r| r.join(theme).join("index.theme").exists()) {
            return theme.to_string();
        }
    }
    "hicolor".into()
}

/// DMS bar widget id → yuck widget call.
fn bar_widget(id: &str) -> Option<&'static str> {
    Some(match id {
        "launcherButton" => "(bar-launcher)",
        "workspaceSwitcher" => "(bar-workspaces)",
        "focusedWindow" => "(bar-focused)",
        "music" | "media" => "(bar-music-slot)",
        "clock" => "(bar-clock)",
        "weather" => "(bar-weather)",
        "systemTray" => "(bar-tray)",
        "clipboard" => "(bar-clipboard)",
        "cpuUsage" => "(bar-cpu)",
        "memUsage" => "(bar-mem)",
        "notificationButton" => "(bar-notifs)",
        "battery" => "(bar-battery)",
        "controlCenterButton" => "(bar-cc)",
        "idleInhibitor" => "(bar-idle)",
        "powerMenuButton" => "(bar-power)",
        _ => return None,
    })
}

impl Eww {
    pub fn new() -> Self {
        let dir = crate::util::runtime_dir().join("eww");
        let bin = std::env::var("DMR_EWW").unwrap_or_else(|_| "eww".into());
        Eww { dir, bin, pid: Mutex::new(None), generation: AtomicU64::new(0), crashes: Mutex::new(Vec::new()) }
    }

    /// Prepare the runtime config directory. Shipped files are symlinked so a
    /// package update is picked up on the next reload; only `_theme.scss` is
    /// generated here.
    pub fn prepare_dir(&self) -> std::io::Result<()> {
        let src = crate::util::share_dir().join("eww");
        std::fs::create_dir_all(&self.dir)?;
        for entry in std::fs::read_dir(&src)? {
            let entry = entry?;
            let name = entry.file_name();
            if name.to_string_lossy().starts_with("_theme") {
                continue;
            }
            let dst = self.dir.join(&name);
            let target = entry.path();
            if std::fs::read_link(&dst).map(|t| t == target).unwrap_or(false) {
                continue;
            }
            let _ = std::fs::remove_file(&dst);
            std::os::unix::fs::symlink(&target, &dst)?;
        }
        // The bar layout comes from the config: only the widgets in use exist.
        let c = crate::app::cfg();
        let section = |name: &str, list: &[String]| -> String {
            let items: Vec<&str> = list.iter().filter_map(|w| bar_widget(w)).collect();
            format!("(defwidget bar-{name} [] (box :space-evenly false :spacing 4 {}))\n", items.join(" "))
        };
        let generated = format!(
            "; generated by dmr\n{}{}{}",
            section("left", &c.bar.left),
            section("center", &c.bar.center),
            section("right", &c.bar.right),
        );
        crate::util::write_if_changed(&self.dir.join("_gen.yuck"), &generated)?;
        Ok(())
    }

    fn cmd(&self) -> tokio::process::Command {
        let mut c = tokio::process::Command::new(&self.bin);
        // --no-daemonize: if our daemon is gone, `eww open` must fail instead
        // of silently starting a stray daemon of its own (each one would keep
        // its own windows and listeners forever).
        c.arg("--no-daemonize").arg("-c").arg(&self.dir).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
        c
    }

    pub async fn start(&self) {
        if let Err(e) = self.prepare_dir() {
            eprintln!("dmr: cannot prepare eww dir: {e}");
        }
        // Make sure a stale instance for this config dir is gone.
        let _ = self.cmd().arg("kill").status().await;
        // A private XDG_CONFIG_HOME shields the shell from the user's GTK CSS
        // and settings (dmr themes GTK apps itself via templates).
        let xdg = crate::util::runtime_dir().join("eww-xdg");
        let _ = std::fs::create_dir_all(xdg.join("gtk-3.0"));
        let _ = crate::util::write_if_changed(&xdg.join("gtk-3.0/gtk.css"), "/* dmr: isolate the shell from user GTK CSS */\n");
        let _ = crate::util::write_if_changed(
            &xdg.join("gtk-3.0/settings.ini"),
            &format!(
                "[Settings]\ngtk-application-prefer-dark-theme=true\ngtk-font-name=Google Sans Flex 11\ngtk-cursor-blink=true\ngtk-icon-theme-name={}\n",
                icon_theme()
            ),
        );
        // The private XDG_CONFIG_HOME above would also hide the user's
        // fontconfig (Home Manager registers per-user fonts there): include
        // it explicitly, plus the fonts shipped with the package.
        // (Loaded through FONTCONFIG_FILE: the system config includes
        // $XDG_CONFIG_HOME/fontconfig/fonts.conf, so that name would recurse.)
        let fc = xdg.join("dmr-fonts.conf");
        let _ = crate::util::write_if_changed(&fc, &fontconfig());
        let mut c = self.cmd();
        c.env("XDG_CONFIG_HOME", &xdg);
        c.env("FONTCONFIG_FILE", &fc);
        // Less memory: no accessibility bridge (extra libraries + a D-Bus
        // connection per process) and no GL (GTK3 widgets are drawn by cairo).
        c.env("NO_AT_BRIDGE", "1").env("GTK_A11Y", "none").env("GDK_GL", "disable");
        c.arg("daemon");
        if std::env::var_os("DMR_EWW_LOG").is_some() {
            c.stderr(Stdio::inherit()).stdout(Stdio::inherit());
        }
        let generation = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        match c.spawn() {
            Ok(mut child) => {
                *self.pid.lock().unwrap() = child.id();
                // Watchdog: if EWW dies on its own, bring the whole UI back.
                tokio::spawn(async move {
                    let status = child.wait().await;
                    let eww = &crate::app::app().eww;
                    if eww.generation.load(Ordering::SeqCst) != generation {
                        return; // stopped or restarted on purpose
                    }
                    let recent = {
                        let mut c = eww.crashes.lock().unwrap();
                        c.retain(|t| t.elapsed() < Duration::from_secs(60));
                        c.push(std::time::Instant::now());
                        c.len()
                    };
                    eprintln!("dmr: eww exited unexpectedly ({status:?})");
                    if recent > 3 {
                        eprintln!("dmr: eww keeps crashing; not restarting it again (see DMR_EWW_LOG=1)");
                        return;
                    }
                    tokio::time::sleep(Duration::from_secs(1)).await;
                    CRASHED.notify_one();
                });
            }
            Err(e) => {
                eprintln!("dmr: cannot start eww ({}): {e}", self.bin);
                return;
            }
        }
        for _ in 0..100 {
            if matches!(self.cmd().arg("ping").status().await, Ok(s) if s.success()) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        eprintln!("dmr: eww did not come up");
    }

    pub async fn open(&self, window: &str, id: &str, screen: Option<&str>, args: &[(&str, String)]) {
        let mut c = self.cmd();
        c.arg("open").arg(window).arg("--id").arg(id);
        if let Some(s) = screen {
            c.arg("--screen").arg(s);
        }
        for (k, v) in args {
            c.arg("--arg").arg(format!("{k}={v}"));
        }
        let _ = c.status().await;
    }

    pub async fn close(&self, ids: &[&str]) {
        if ids.is_empty() {
            return;
        }
        let _ = self.cmd().arg("close").args(ids).status().await;
    }

    pub async fn reload(&self) {
        let _ = self.prepare_dir();
        let _ = self.cmd().arg("reload").status().await;
    }

    pub async fn kill(&self) {
        // Tell the watchdog this exit is intended.
        self.generation.fetch_add(1, Ordering::SeqCst);
        let _ = self.cmd().arg("kill").status().await;
        if let Some(pid) = self.pid.lock().unwrap().take() {
            unsafe { libc::kill(pid as libc::pid_t, libc::SIGTERM) };
        }
    }
}
