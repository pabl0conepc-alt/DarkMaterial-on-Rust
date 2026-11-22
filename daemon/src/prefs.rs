//! The Settings window (DMS "Settings"): page navigation with search, the bar
//! widget editor, the keyboard shortcut list read from the driftwm config,
//! installed fonts, theme pickers. Everything is published on the `prefs`
//! topic; values themselves live in the `cfg` topic and are changed with
//! `dmr config set|step|cycle|list-*`.

use std::sync::Mutex;

use anyhow::{Result, bail};
use serde_json::{Value, json};

use crate::app::app;

/// (id, title, subtitle, material icon, colour group, extra search words).
/// Groups follow DMS's category colours; a group change draws a gap.
const PAGES: &[(&str, &str, &str, &str, u8, &str)] = &[
    ("personalization", "Wallpaper & colors", "Theme, palette, dark mode", "wallpaper", 0, "light dark scheme material you dynamic"),
    ("surfaces", "Interface style", "Transparency, corners", "layers", 0, "opacity radius rounding blur"),
    ("typography", "Fonts & motion", "Fonts, scale, animations", "text_fields", 0, "font family size speed duration"),
    ("bar", "Bar", "Layout, position, appearance", "web_asset", 1, "top bottom floating scroll workspaces"),
    ("barwidgets", "Bar widgets", "Add, reorder, configure", "widgets", 1, "clock cpu memory tray media"),
    ("launcher", "Launcher", "Style, search, hidden apps", "apps", 1, "spotlight search results"),
    ("dash", "Dashboard", "Tabs and weather", "space_dashboard", 1, "media weather wallpapers tabs"),
    ("notifications", "Notifications", "Popups, do not disturb", "notifications", 2, "dnd timeout popup"),
    ("osd", "On-screen displays", "Volume, brightness", "picture_in_picture", 2, "osd volume brightness"),
    ("sound", "Sound & media", "Audio, media player", "volume_up", 2, "output input speaker microphone player"),
    ("displays", "Displays", "Night mode, brightness", "desktop_windows", 3, "night light gamma temperature"),
    ("power", "Power & battery", "Power profile, battery", "power_settings_new", 3, "battery profile performance saver"),
    ("network", "Network", "Wi-Fi, Bluetooth", "wifi", 4, "wifi bluetooth wireless"),
    ("keybinds", "Keyboard shortcuts", "driftwm keybindings", "keyboard", 4, "keys binds hotkeys"),
    ("apps", "Default apps", "Terminal, files, mixer", "apps_outage", 4, "terminal file manager audio"),
    ("time", "Time & weather", "Clock format, location", "schedule", 4, "clock 12h 24h weather location units"),
    ("about", "About", "Version, credits", "info", 5, "version credits license"),
];

/// Sub-pages: (id, parent).
const SUBPAGES: &[(&str, &str, &str)] = &[("theme", "personalization", "Theme & colors")];

/// Bar widget catalogue: (id, name, description, icon) — DMS's wording.
pub const WIDGETS: &[(&str, &str, &str, &str)] = &[
    ("launcherButton", "App launcher", "Quick access to application launcher", "apps"),
    ("workspaceSwitcher", "Workspace switcher", "Shows current workspace and allows switching", "view_module"),
    ("focusedWindow", "Focused window", "Display currently focused application title", "select_window"),
    ("music", "Media", "Now playing and media controls", "music_note"),
    ("clock", "Clock", "Current time and date display", "schedule"),
    ("weather", "Weather", "Current weather conditions and temperature", "wb_sunny"),
    ("systemTray", "System tray", "System notification area icons", "widgets"),
    ("clipboard", "Clipboard manager", "Access clipboard history", "content_paste"),
    ("cpuUsage", "CPU usage", "CPU usage indicator", "memory"),
    ("memUsage", "Memory usage", "Memory usage indicator", "memory_alt"),
    ("notificationButton", "Notification center", "Access to notifications and do not disturb", "notifications"),
    ("battery", "Battery", "Battery level and power management", "battery_full"),
    ("controlCenterButton", "Control center", "Access to system controls and settings", "tune"),
    ("idleInhibitor", "Idle inhibitor", "Prevent screen timeout", "coffee"),
    ("powerMenuButton", "Power", "Power menu", "power_settings_new"),
];

/// Fonts offered in "Fonts & motion" when installed.
const FONTS: &[&str] = &[
    "Google Sans Flex", "Inter", "Rubik", "Roboto", "Noto Sans", "Cantarell", "Fira Sans", "Open Sans", "Ubuntu", "Lato",
    "Montserrat", "IBM Plex Sans", "Source Sans 3", "DejaVu Sans", "JetBrains Mono",
];

/// The Settings window was opened: (re)load the current page.
static LOAD: tokio::sync::Notify = tokio::sync::Notify::const_new();

pub fn request_load() {
    LOAD.notify_one();
}

pub async fn loader() {
    loop {
        LOAD.notified().await;
        let p = app().prefs.page();
        let p = if p.is_empty() { "personalization".to_string() } else { p };
        let _ = app().prefs.command("page", &[p]).await;
    }
}

#[derive(Default)]
struct State {
    page: String,
    open: String,
    search: String,
    kb_search: String,
    pick: String,
    fonts: Vec<String>,
}

#[derive(Default)]
pub struct Prefs {
    st: Mutex<State>,
}

fn widget_meta(id: &str) -> (&'static str, &'static str, &'static str) {
    WIDGETS.iter().find(|w| w.0 == id).map(|w| (w.1, w.2, w.3)).unwrap_or(("Unknown", "", "help"))
}

/// `mod+shift+1` → `Super + Shift + 1`.
fn pretty_keys(k: &str) -> String {
    k.split('+')
        .map(|p| match p.to_lowercase().as_str() {
            "mod" | "super" => "Super".to_string(),
            "ctrl" | "control" => "Ctrl".to_string(),
            "alt" => "Alt".to_string(),
            "shift" => "Shift".to_string(),
            "return" => "Enter".to_string(),
            "page_up" => "PgUp".to_string(),
            "page_down" => "PgDn".to_string(),
            "comma" => ",".to_string(),
            "period" => ".".to_string(),
            o if o.starts_with("xf86") => p[4..].to_string(),
            _ => {
                let mut c = p.chars();
                c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
            }
        })
        .collect::<Vec<_>>()
        .join(" + ")
}

/// Human description of a driftwm keybinding action.
fn describe(action: &str) -> String {
    let a = action.trim();
    let known: &[(&str, &str)] = &[
        ("spawn dmr toggle launcher", "Application launcher"),
        ("spawn dmr-clipboard", "Clipboard manager"),
        ("spawn dmr toggle dash media", "Dash: media"),
        ("spawn dmr toggle dash overview", "Dash"),
        ("spawn dmr toggle dash wallpaper", "Browse wallpapers"),
        ("spawn dmr toggle controlcenter", "Control center"),
        ("spawn dmr toggle powermenu", "Power menu"),
        ("spawn dmr toggle notifcenter", "Notification center"),
        ("spawn dmr settings", "Settings"),
        ("spawn dmr lock", "Lock screen"),
        ("spawn dmr night", "Night mode"),
        ("spawn dmr idle-inhibit", "Idle inhibit"),
        ("spawn dmr-emoji", "Emoji picker"),
        ("spawn dmr notifs clear", "Clear notifications"),
        ("spawn dmr quit", "Quit the shell"),
        ("spawn dmr volume up", "Volume up"),
        ("spawn dmr volume down", "Volume down"),
        ("spawn dmr volume mute", "Mute"),
        ("spawn dmr mic mute", "Mute microphone"),
        ("spawn dmr brightness up", "Brightness up"),
        ("spawn dmr brightness down", "Brightness down"),
        ("spawn dmr media play-pause", "Play / pause"),
        ("spawn dmr media next", "Next track"),
        ("spawn dmr media previous", "Previous track"),
        ("spawn dmr media stop", "Stop media"),
        ("spawn dmr screenshot region", "Screenshot: region"),
        ("spawn dmr screenshot full", "Screenshot: full screen"),
        ("spawn dmr ws-scroll down", "Next workspace"),
        ("spawn dmr ws-scroll up", "Previous workspace"),
        ("close-window", "Close window"),
        ("toggle-fullscreen", "Toggle fullscreen"),
        ("quit", "Quit driftwm"),
        ("home-toggle", "Go to canvas origin"),
        ("zoom-to-fit", "Overview (zoom to fit)"),
        ("toggle-pin-to-screen", "Pin window to screen"),
        ("center-window", "Center window"),
        ("cycle-windows forward", "Next window"),
        ("cycle-windows backward", "Previous window"),
    ];
    if let Some((_, d)) = known.iter().find(|(k, _)| *k == a) {
        return d.to_string();
    }
    if let Some(n) = a.strip_prefix("go-to-bookmark ") {
        return format!("Go to workspace {n}");
    }
    if let Some(n) = a.strip_prefix("move-to-bookmark ") {
        return format!("Move window to workspace {n}");
    }
    if let Some(n) = a.strip_prefix("set-bookmark ") {
        return format!("Save workspace {n} here");
    }
    if let Some(c) = a.strip_prefix("exec ").or_else(|| a.strip_prefix("spawn ")) {
        return format!("Run {c}");
    }
    a.replace('-', " ")
}

fn keybinds() -> Vec<(String, String, String)> {
    let path = crate::util::expand(&crate::app::cfg().driftwm.config_path);
    let text = std::fs::read_to_string(path).unwrap_or_default();
    let mut in_binds = false;
    let mut out = vec![];
    for line in text.lines() {
        let l = line.trim();
        if l.starts_with('[') {
            in_binds = l == "[keybindings]";
            continue;
        }
        if !in_binds || l.starts_with('#') {
            continue;
        }
        let Some((k, v)) = l.split_once('=') else { continue };
        let k = k.trim().trim_matches('"');
        let v = v.split('#').next().unwrap_or("").trim().trim_matches('"');
        if k.is_empty() || v.is_empty() || v == "none" {
            continue;
        }
        out.push((pretty_keys(k), describe(v), v.to_string()));
    }
    out
}

impl Prefs {
    pub fn page(&self) -> String {
        self.st.lock().unwrap().page.clone()
    }

    pub fn publish(&self) {
        let st = self.st.lock().unwrap();
        let q = st.search.to_lowercase();
        let page = if st.page.is_empty() { "personalization".to_string() } else { st.page.clone() };
        let mut last_group = 255u8;
        let pages: Vec<Value> = PAGES
            .iter()
            .map(|(id, title, sub, icon, group, kw)| {
                let vis = q.is_empty()
                    || title.to_lowercase().contains(&q)
                    || sub.to_lowercase().contains(&q)
                    || kw.contains(q.as_str());
                let gap = *group != last_group && last_group != 255;
                last_group = *group;
                json!({ "id": id, "title": title, "sub": sub, "icon": icon, "group": group, "vis": vis, "gap": gap })
            })
            .collect();
        let (title, parent) = SUBPAGES
            .iter()
            .find(|s| s.0 == page)
            .map(|s| (s.2.to_string(), s.1.to_string()))
            .or_else(|| PAGES.iter().find(|p| p.0 == page).map(|p| (p.1.to_string(), String::new())))
            .unwrap_or_default();
        let nav = if parent.is_empty() { page.clone() } else { parent.clone() };

        let c = crate::app::cfg();
        let section = |list: &[String]| -> Vec<Value> {
            list.iter()
                .enumerate()
                .map(|(i, id)| {
                    let (name, desc, icon) = widget_meta(id);
                    json!({ "i": i, "id": id, "name": name, "desc": desc, "icon": icon, "on": !c.bar.hidden.contains(id) })
                })
                .collect()
        };
        let used: Vec<&String> = c.bar.left.iter().chain(&c.bar.center).chain(&c.bar.right).collect();
        let available: Vec<Value> = WIDGETS
            .iter()
            .filter(|w| !used.iter().any(|u| u.as_str() == w.0))
            .map(|w| json!({ "id": w.0, "name": w.1, "desc": w.2, "icon": w.3 }))
            .collect();

        let kq = st.kb_search.to_lowercase();
        let binds: Vec<Value> = if page == "keybinds" {
            keybinds()
                .into_iter()
                .filter(|(k, d, a)| kq.is_empty() || k.to_lowercase().contains(&kq) || d.to_lowercase().contains(&kq) || a.contains(&kq))
                .map(|(k, d, a)| json!({ "keys": k.split(" + ").collect::<Vec<_>>(), "desc": d, "action": a }))
                .collect()
        } else {
            vec![]
        };

        let stock_all: Value = serde_json::from_str(include_str!("../../schemes/dms/stock.json")).unwrap_or_default();
        let theme = app().theme.current_scheme_id();
        let stock: Vec<Value> = crate::theme::stock_names()
            .iter()
            .map(|n| {
                let mut name = n.clone();
                if let Some(f) = name.get_mut(0..1) {
                    f.make_ascii_uppercase();
                }
                json!({ "id": n, "name": name, "color": stock_all["DARK"][n]["primary"], "current": theme == format!("stock {n}") })
            })
            .collect();
        let schemes: Vec<Value> = if page == "theme" {
            crate::theme::list_schemes()
                .into_iter()
                .filter(|(n, _)| n != "dynamic")
                .map(|(n, f)| {
                    let id = format!("{n} {f}");
                    json!({ "name": n, "flavour": f, "label": if f == "default" { n.clone() } else { format!("{n} · {f}") },
                            "colours": crate::theme::scheme_preview(&n, &f), "current": theme == id })
                })
                .collect()
        } else {
            vec![]
        };
        let variant = app().theme.current_variant();
        let variants: Vec<Value> = crate::theme::VARIANTS
            .iter()
            .map(|(id, name, _, _)| json!({ "id": id, "name": name, "current": *id == variant }))
            .collect();

        let mut fonts = st.fonts.clone();
        if !fonts.contains(&c.appearance.font_family) {
            fonts.insert(0, c.appearance.font_family.clone());
        }
        let theme_label = {
            let mut it = theme.split_whitespace();
            let (n, f) = (it.next().unwrap_or(""), it.next().unwrap_or(""));
            let cap = |s: &str| {
                let mut c = s.chars();
                c.next().map(|x| x.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
            };
            match n {
                "dynamic" => "Material You".to_string(),
                "stock" => cap(f),
                _ if f == "default" || f.is_empty() => cap(n),
                _ => format!("{} {}", cap(n), cap(f)),
            }
        };
        let wall = app().theme.current_wallpaper();
        let v = json!({
            "page": page, "nav": nav, "title": title, "parent": parent,
            "open": st.open, "search": st.search, "kbSearch": st.kb_search, "pick": st.pick,
            "pages": pages,
            "left": section(&c.bar.left), "center": section(&c.bar.center), "right": section(&c.bar.right),
            "available": available,
            "binds": binds,
            "stock": stock, "schemes": schemes, "variants": variants,
            "mode": if theme.starts_with("dynamic") { "auto" } else if theme.starts_with("stock") { "generic" } else { "browse" },
            "themeLabel": theme_label,
            "fonts": fonts,
            "wallName": std::path::Path::new(&wall).file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default(),
            "wallThumb": if wall.is_empty() { String::new() } else { crate::theme::thumbnail_path(std::path::Path::new(&wall)).to_string_lossy().into_owned() },
            "version": env!("CARGO_PKG_VERSION"),
            "initial": crate::info::username().chars().next().map(|c| c.to_uppercase().to_string()).unwrap_or_default(),
        });
        drop(st);
        app().hub.publish("prefs", &v);
    }

    async fn load_fonts(&self) {
        if !self.st.lock().unwrap().fonts.is_empty() {
            return;
        }
        // Same font configuration EWW uses.
        let fc = crate::util::runtime_dir().join("eww-xdg/dmr-fonts.conf");
        let out = tokio::process::Command::new("fc-list")
            .arg(":")
            .arg("family")
            .env("FONTCONFIG_FILE", fc)
            .output()
            .await
            .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
            .unwrap_or_default();
        let installed: Vec<&str> = out.lines().flat_map(|l| l.split(',')).map(str::trim).collect();
        let fonts: Vec<String> = FONTS.iter().filter(|f| installed.contains(f)).map(|f| f.to_string()).collect();
        self.st.lock().unwrap().fonts = if fonts.is_empty() { vec!["Google Sans Flex".into()] } else { fonts };
    }

    pub async fn command(&self, sub: &str, a: &[String]) -> Result<String> {
        let arg = |i: usize| a.get(i).map(String::as_str).unwrap_or("");
        match sub {
            "page" => {
                let p = arg(0);
                if !PAGES.iter().any(|x| x.0 == p) && !SUBPAGES.iter().any(|x| x.0 == p) {
                    bail!("unknown settings page {p:?}");
                }
                {
                    let mut st = self.st.lock().unwrap();
                    st.page = p.to_string();
                    st.open.clear();
                    st.pick.clear();
                }
                if p == "typography" {
                    self.load_fonts().await;
                }
                if p == "personalization" || p == "theme" {
                    crate::theme::publish_walls().await;
                }
                if p == "network" {
                    app().net.set_list_demand(true);
                    let _ = app().net.command("scan", &[]).await;
                }
            }
            // Toggle an inline drop-down (one open at a time).
            "open" => {
                let mut st = self.st.lock().unwrap();
                st.open = if st.open == arg(0) { String::new() } else { arg(0).to_string() };
            }
            "search" => self.st.lock().unwrap().search = arg(0).chars().take(64).collect(),
            "kbsearch" => self.st.lock().unwrap().kb_search = arg(0).chars().take(64).collect(),
            "pick" => {
                let mut st = self.st.lock().unwrap();
                st.pick = if st.pick == arg(0) { String::new() } else { arg(0).to_string() };
            }
            // Bar widgets: add <section> <id> | move <section> <i> up|down |
            // remove <section> <i> | hide <id> | reset
            "add" => {
                let (sec, id) = (arg(0), arg(1));
                if !matches!(sec, "left" | "center" | "right") || !WIDGETS.iter().any(|w| w.0 == id) {
                    bail!("prefs add left|center|right <widget>");
                }
                self.st.lock().unwrap().pick.clear();
                crate::settings::command("list-add", &format!("bar.{sec}"), &[id]).await?;
            }
            "move" => {
                crate::settings::command("list-move", &format!("bar.{}", arg(0)), &[arg(1), arg(2)]).await?;
            }
            "remove" => {
                crate::settings::command("list-remove", &format!("bar.{}", arg(0)), &[arg(1)]).await?;
            }
            "hide" => {
                crate::settings::command("list-toggle", "bar.hidden", &[arg(0)]).await?;
            }
            "reset" => {
                let d = crate::config::Bar::default();
                for (k, v) in [("left", &d.left), ("center", &d.center), ("right", &d.right)] {
                    crate::settings::write(&format!("bar.{k}"), json!(v)).await?;
                }
                crate::settings::write("bar.hidden", json!([])).await?;
            }
            // Free text for a whitelisted string setting, read from stdin.
            "text" => {
                const TEXT_KEYS: &[&str] = &[
                    "general.terminal", "general.explorer", "general.audio", "services.weatherLocation", "services.defaultPlayer",
                ];
                let (key, val) = (arg(0), arg(1));
                if !TEXT_KEYS.contains(&key) {
                    bail!("not a text setting: {key}");
                }
                let val: String = val.chars().filter(|c| !c.is_control()).take(200).collect();
                crate::settings::write(key, Value::String(val.trim().to_string())).await?;
            }
            _ => bail!("prefs page|open|search|kbsearch|pick|add|move|remove|hide|reset|text"),
        }
        self.publish();
        Ok(String::new())
    }
}
