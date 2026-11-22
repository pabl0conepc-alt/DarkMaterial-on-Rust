//! Panel state machine for the DankMaterialShell layout.
//!
//! * Heavy panels (control center, dash, notification center) are built once
//!   at startup and stay mapped with their revealer collapsed: opening them is
//!   only a reveal, never a rebuild of the widget tree.
//! * Panels that need the keyboard (spotlight launcher, power menu) are mapped
//!   when opened and unmapped after the closing animation, so they never keep
//!   the keyboard while hidden.
//! * A transparent "catcher" window under the panels closes them on an outside
//!   click; for modals (launcher, notification center, power menu) it also dims
//!   the screen like DMS does.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;

use anyhow::{Result, bail};
use serde_json::{Value, json};
use tokio::sync::Notify;

use crate::app::app;

/// Centered modals: dim the screen.
const MODAL: &[&str] = &["launcher", "notifcenter", "powermenu"];
/// Bar popouts: close on outside click, no dim.
const POPOUT: &[&str] = &["controlcenter", "dash"];
const ALL: &[&str] = &["launcher", "notifcenter", "powermenu", "controlcenter", "dash", "osd", "settings"];
/// Kept mapped (collapsed) between uses: building them is the expensive part.
/// Only panels whose collapsed revealer has zero height can stay mapped: a
/// collapsed *crossfade* revealer keeps its full size and the invisible
/// surface would swallow clicks (the notification center fades, so it is
/// mapped on open instead).
const PERSISTENT: &[&str] = &["controlcenter", "dash"];
/// Revealer transition length used in the yuck (keep in sync).
const ANIM_MS: u64 = 250;

#[derive(Default, Clone)]
struct Panel {
    shown: bool,
    mapped: bool,
    generation: u64,
}

#[derive(Default)]
struct State {
    panels: HashMap<&'static str, Panel>,
    catcher_mapped: bool,
    dash_tab: u8,
    outputs: Vec<(String, i32, i32, bool)>, // name, width, height, active
    fullscreen: bool,
    static_open: Vec<String>,
    misc: serde_json::Map<String, Value>,
    osd_gen: u64,
    power_sel: i64,
}

#[derive(Default)]
pub struct Ui {
    st: Mutex<State>,
    quit: Notify,
}

fn key(name: &str) -> Option<&'static str> {
    let name = match name {
        // DMS IPC names.
        "spotlight" => "launcher",
        "control-center" | "cc" => "controlcenter",
        "notifications" | "notification-center" => "notifcenter",
        "power" | "session" => "powermenu",
        "dashboard" | "dankdash" => "dash",
        "prefs" => "settings",
        n => n,
    };
    ALL.iter().copied().find(|p| *p == name)
}

/// Power menu entries, in DMS order: (id, label, icon, key).
pub const POWER: &[(&str, &str, &str, &str)] = &[
    ("reboot", "Reboot", "restart_alt", "R"),
    ("logout", "Log out", "logout", "X"),
    ("poweroff", "Power off", "power_settings_new", "P"),
    ("lock", "Lock", "lock", "L"),
    ("suspend", "Suspend", "bedtime", "S"),
    ("restart", "Restart Shell", "refresh", "D"),
];

impl Ui {
    pub fn publish(&self) {
        let st = self.st.lock().unwrap();
        let mut v = json!({
            "dashTab": st.dash_tab,
            "fullscreen": st.fullscreen,
            "scrim": false,
            "modal": MODAL.iter().any(|p| st.panels.get(p).map(|x| x.shown).unwrap_or(false)),
            "osdKind": "volume",
            "powerSel": st.power_sel,
            "ccPage": "",
            "wifiAskPassword": "",
            "gamemode": false,
        });
        for p in ALL {
            v[*p] = Value::Bool(st.panels.get(p).map(|x| x.shown).unwrap_or(false));
        }
        for (k, val) in &st.misc {
            v[k] = val.clone();
        }
        drop(st);
        app().hub.publish("ui", &v);
    }

    pub fn set_misc(&self, k: &str, v: Value) {
        self.st.lock().unwrap().misc.insert(k.to_string(), v);
        self.publish();
    }

    pub fn request_quit(&self) {
        self.quit.notify_one();
    }
    pub async fn quit_requested(&self) {
        self.quit.notified().await
    }

    pub fn is_shown(&self, name: &str) -> bool {
        self.st.lock().unwrap().panels.get(name).map(|p| p.shown).unwrap_or(false)
    }

    pub fn active_output(&self) -> Option<String> {
        let st = self.st.lock().unwrap();
        // Without compositor output names (not driftwm), the first monitor.
        Some(st.outputs.iter().find(|o| o.3).or(st.outputs.first()).map(|o| o.0.clone()).filter(|n| !n.is_empty()).unwrap_or_else(|| "0".into()))
    }

    pub fn active_height_pub(&self) -> i32 {
        self.active_size().1
    }

    fn active_size(&self) -> (i32, i32) {
        let st = self.st.lock().unwrap();
        st.outputs
            .iter()
            .find(|o| o.3)
            .or(st.outputs.first())
            .map(|o| (o.1, o.2))
            .filter(|s| s.0 > 0 && s.1 > 0)
            .unwrap_or((1920, 1080))
    }

    /// Dash tabs: Overview, Media, Wallpapers, Weather.
    pub fn dash_tab_step(&self, dir: i32) {
        let cur = self.st.lock().unwrap().dash_tab as i32;
        self.set_dash_tab((cur + dir).clamp(0, 3) as u8);
    }

    pub fn set_dash_tab(&self, t: u8) {
        self.st.lock().unwrap().dash_tab = t.min(3);
        self.publish();
        self.update_demand();
    }

    fn update_demand(&self) {
        let (dash, cc, tab) = {
            let st = self.st.lock().unwrap();
            let s = |p: &str| st.panels.get(p).map(|x| x.shown).unwrap_or(false);
            (s("dash"), s("controlcenter"), st.dash_tab)
        };
        let hub = &app().hub;
        // Full resource monitor only while the control center (disk) is open.
        hub.set_demand("sys", cc);
        hub.set_demand("media-pos", dash && tab == 1);
        crate::clock::WAKE.notify_one();
        hub.set_demand("info", dash || cc);
        app().sys.wake();
        app().info.wake();
        if dash && tab == 2 {
            tokio::spawn(crate::theme::publish_walls());
        }
        app().net.set_list_demand(cc && self.st.lock().unwrap().misc.get("ccPage").and_then(Value::as_str) == Some("network"));
    }

    // ---- static windows (the bar), one per output ----

    pub async fn set_outputs(&self, outs: Vec<(String, i32, i32, bool)>) {
        let changed = {
            let mut st = self.st.lock().unwrap();
            let names: Vec<_> = outs.iter().map(|o| (&o.0, o.1, o.2)).collect();
            let old: Vec<_> = st.outputs.iter().map(|o| (&o.0, o.1, o.2)).collect();
            let changed = names != old;
            st.outputs = outs;
            changed
        };
        if changed {
            self.open_static().await;
        }
    }

    pub fn is_fullscreen(&self) -> bool {
        self.st.lock().unwrap().fullscreen
    }

    pub async fn set_fullscreen(&self, fs: bool) {
        {
            let mut st = self.st.lock().unwrap();
            if st.fullscreen == fs {
                return;
            }
            st.fullscreen = fs;
        }
        self.publish();
        if fs {
            self.close_all().await;
        }
        self.open_static().await;
    }

    /// (Re)open the bar on every output and pre-build the persistent panels.
    pub async fn open_static(&self) {
        let eww = &app().eww;
        let (outs, fs, old) = {
            let st = self.st.lock().unwrap();
            (st.outputs.clone(), st.fullscreen, st.static_open.clone())
        };
        let old_refs: Vec<&str> = old.iter().map(String::as_str).collect();
        eww.close(&old_refs).await;
        let mut opened = vec![];
        if !fs {
            let outs = if outs.is_empty() { vec![(String::new(), 0, 0, true)] } else { outs };
            let c = crate::app::cfg();
            for (i, o) in outs.iter().enumerate() {
                let screen = if o.0.is_empty() { Some("0") } else { Some(o.0.as_str()) };
                let id = format!("bar-{i}");
                let args = vec![
                    ("monitor", o.0.clone()),
                    ("floating", c.bar.floating.to_string()),
                    ("bottom", (c.bar.position == "bottom").to_string()),
                ];
                eww.open("bar", &id, screen, &args).await;
                opened.push(id);
            }
        }
        self.st.lock().unwrap().static_open = opened;
        for k in PERSISTENT {
            self.premap(k).await;
        }
        if app().notifs.has_popups() {
            app().notifs.remap().await;
        }
    }

    /// Build a persistent panel's window now, collapsed, on the active output.
    pub async fn premap(&self, k: &'static str) {
        if self.is_shown(k) {
            return;
        }
        let screen = self.active_output();
        app().eww.close(&[k]).await;
        let args = self.window_args(k);
        app().eww.open(k, k, screen.as_deref(), &args).await;
        self.st.lock().unwrap().panels.entry(k).or_default().mapped = true;
    }

    fn window_args(&self, k: &str) -> Vec<(&'static str, String)> {
        let (_w, h) = self.active_size();
        match k {
            // Notification center: a centred modal, at most 70% of the screen.
            "notifcenter" => vec![("h", ((h as f64 * 0.62) as i32).clamp(360, 760).to_string())],
            // Bar popouts open from the bar's edge.
            "controlcenter" | "dash" => vec![("bottom", (crate::app::cfg().bar.position == "bottom").to_string())],
            // Settings: DMS opens it as a 960 px wide window.
            "settings" => {
                let (w, _) = self.active_size();
                vec![("w", 960.min(w - 40).to_string()), ("h", ((h as f64 * 0.86) as i32).clamp(480, 1000).to_string())]
            }
            _ => vec![],
        }
    }

    pub async fn toggle(&self, name: &str) -> Result<String> {
        let Some(k) = key(name) else { bail!("unknown panel {name:?}") };
        if self.is_shown(k) {
            self.hide(k).await
        } else if k == "osd" {
            self.osd("volume").await
        } else {
            self.show(k).await
        }
        Ok(String::new())
    }

    pub async fn open(&self, name: &str) -> Result<String> {
        let Some(k) = key(name) else { bail!("unknown panel {name:?}") };
        if k == "osd" {
            self.osd("volume").await;
        } else {
            self.show(k).await;
        }
        Ok(String::new())
    }

    pub async fn close(&self, name: &str) -> Result<String> {
        let Some(k) = key(name) else { bail!("unknown panel {name:?}") };
        self.hide(k).await;
        Ok(String::new())
    }

    pub async fn close_all(&self) {
        // The Settings window is a regular window in DMS: only its own
        // close button / Esc / `dmr close settings` close it.
        for p in ALL.iter().filter(|p| **p != "settings") {
            if self.is_shown(p) {
                self.hide(p).await;
            }
        }
    }

    pub async fn show(&self, k: &'static str) {
        if self.st.lock().unwrap().fullscreen && k != "osd" && k != "powermenu" {
            return;
        }
        if MODAL.contains(&k) || POPOUT.contains(&k) {
            for other in MODAL.iter().chain(POPOUT) {
                if *other != k && self.is_shown(other) {
                    self.hide(other).await;
                }
            }
        }
        let need_map = {
            let mut st = self.st.lock().unwrap();
            if k == "powermenu" {
                st.power_sel = 0;
            }
            if k == "controlcenter" {
                st.misc.insert("ccPage".into(), Value::from(""));
            }
            let p = st.panels.entry(k).or_default();
            p.generation += 1;
            if p.shown && p.mapped {
                return;
            }
            let need = !p.mapped;
            p.mapped = true;
            need
        };
        if k == "launcher" {
            app().launcher.reset();
        }
        if need_map {
            let screen = self.active_output();
            let args = self.window_args(k);
            app().eww.open(k, k, screen.as_deref(), &args).await;
            // Let GTK map the window with the revealer collapsed first.
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        self.st.lock().unwrap().panels.entry(k).or_default().shown = true;
        self.sync_catcher().await;
        self.publish();
        self.update_demand();
        if k == "controlcenter" {
            tokio::spawn(async { app().bat.refresh_profile().await });
        }
        if k == "notifcenter" {
            app().notifs.hide_all_popups();
        }
        if k == "settings" {
            // Loaded by prefs::loader (loading a page can write settings,
            // which can open the OSD: no direct call back from here).
            crate::prefs::request_load();
        }
    }

    pub async fn hide(&self, k: &'static str) {
        let generation = {
            let mut st = self.st.lock().unwrap();
            let p = st.panels.entry(k).or_default();
            if !p.shown && !p.mapped {
                return;
            }
            p.shown = false;
            p.generation += 1;
            p.generation
        };
        self.sync_catcher().await;
        self.publish();
        self.update_demand();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(ANIM_MS + 50)).await;
            let ui = &app().ui;
            let unmap = {
                let mut st = ui.st.lock().unwrap();
                let p = st.panels.entry(k).or_default();
                if p.generation == generation && !p.shown && p.mapped && !PERSISTENT.contains(&k) {
                    p.mapped = false;
                    true
                } else {
                    false
                }
            };
            if unmap {
                app().eww.close(&[k]).await;
            }
        });
    }

    /// Transparent full-screen window under the panels: closes them on an
    /// outside click (DMS's click-outside behaviour), dimming for modals.
    async fn sync_catcher(&self) {
        let (want, mapped, dim) = {
            let st = self.st.lock().unwrap();
            let shown = |p: &str| st.panels.get(p).map(|x| x.shown).unwrap_or(false);
            (
                MODAL.iter().chain(POPOUT).any(|p| shown(p)),
                st.catcher_mapped,
                MODAL.iter().any(|p| shown(p)),
            )
        };
        self.st.lock().unwrap().misc.insert("scrim".into(), Value::Bool(dim));
        if want && !mapped {
            let screen = self.active_output();
            app().eww.open("catcher", "catcher", screen.as_deref(), &[]).await;
            self.st.lock().unwrap().catcher_mapped = true;
        } else if !want && mapped {
            self.st.lock().unwrap().catcher_mapped = false;
            app().eww.close(&["catcher"]).await;
        }
    }

    // ---- hover (kept for compatibility with bar hover hooks) ----

    pub async fn hover(&self, what: &str, _extra: &str) {
        if what == "osd" {
            self.st.lock().unwrap().osd_gen += 1;
        }
    }

    pub async fn unhover(&self, what: &str) {
        if what == "osd" {
            let kind = self.st.lock().unwrap().misc.get("osdKind").and_then(Value::as_str).unwrap_or("volume").to_string();
            self.osd(&kind).await;
        }
    }

    // ---- power menu keyboard ----

    pub async fn power_key(&self, key: &str) -> Result<String> {
        let n = POWER.len() as i64;
        let action = {
            let mut st = self.st.lock().unwrap();
            match key {
                "Up" | "ISO_Left_Tab" | "C-k" => {
                    st.power_sel = (st.power_sel - 1).rem_euclid(n);
                    None
                }
                "Down" | "Tab" | "C-j" => {
                    st.power_sel = (st.power_sel + 1).rem_euclid(n);
                    None
                }
                "Escape" => Some("close".to_string()),
                "Return" | "accept" => Some(POWER[st.power_sel as usize].0.to_string()),
                k => {
                    // Typed letters arrive as the hidden input's whole text.
                    let k = k.trim().chars().last().map(|c| c.to_ascii_uppercase().to_string()).unwrap_or_default();
                    POWER.iter().find(|p| p.3 == k).map(|p| p.0.to_string())
                }
            }
        };
        self.publish();
        match action.as_deref() {
            None => Ok(String::new()),
            Some("close") => {
                self.hide("powermenu").await;
                Ok(String::new())
            }
            Some(a) => self.power_action(a).await,
        }
    }

    pub async fn power_action(&self, a: &str) -> Result<String> {
        self.close_all().await;
        match a {
            "restart" => {
                tokio::spawn(async {
                    tokio::time::sleep(Duration::from_millis(300)).await;
                    app().ui.restart_shell().await;
                });
                Ok(String::new())
            }
            "lock" => {
                crate::server::lock();
                Ok(String::new())
            }
            "poweroff" => crate::server::session("shutdown").await,
            other => crate::server::session(other).await,
        }
    }

    /// Restart EWW from scratch (power menu "Restart Shell", or EWW died).
    pub async fn restart_shell(&self) {
        let a = app();
        a.eww.kill().await;
        a.eww.start().await;
        {
            let mut st = self.st.lock().unwrap();
            st.panels.clear();
            st.catcher_mapped = false;
            st.static_open.clear();
        }
        self.publish();
        self.open_static().await;
    }

    // ---- OSD ----

    pub async fn osd(&self, kind: &str) {
        if !crate::app::cfg().osd.enabled || self.is_shown("powermenu") {
            return;
        }
        let generation = {
            let mut st = self.st.lock().unwrap();
            st.osd_gen += 1;
            st.misc.insert("osdKind".into(), Value::from(kind));
            st.osd_gen
        };
        if !self.is_shown("osd") {
            self.show("osd").await;
        } else {
            self.publish();
        }
        let delay = crate::app::cfg().osd.hide_delay;
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(delay)).await;
            let ui = &app().ui;
            if ui.st.lock().unwrap().osd_gen == generation {
                ui.hide("osd").await;
            }
        });
    }
}
