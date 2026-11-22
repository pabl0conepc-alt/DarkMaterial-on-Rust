//! Wall clock and media position for the UI, with as few wake-ups as possible.
//!
//! * `clock`: published once per minute, exactly on the minute boundary (the
//!   bar and the dash only show hours and minutes). EWW's `EWW_TIME` would
//!   wake every widget that uses it once per second instead.
//! * `mpos`: the playing track's position, ticking once per second only while
//!   the dash Media tab is on screen and something is playing.

use std::time::Duration;

use serde_json::{Value, json};

use crate::app::app;

/// Woken when the dash starts or stops showing the media position.
pub static WAKE: tokio::sync::Notify = tokio::sync::Notify::const_new();
/// Woken after resume: tokio's timers use the monotonic clock, which stops
/// while suspended, so the minute timer would be late by the time slept.
static RESUMED: tokio::sync::Notify = tokio::sync::Notify::const_new();

/// logind `PrepareForSleep(false)` = the machine just woke up.
async fn watch_resume() {
    use futures_util::StreamExt;
    let Some(conn) = app().system_bus.get() else { return };
    let Ok(rule) = zbus::MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .interface("org.freedesktop.login1.Manager")
        .member("PrepareForSleep")
        .build()
    else {
        return;
    };
    let Ok(mut stream) = crate::dbus::signals_rule(conn, rule).await else { return };
    while let Some(Ok(msg)) = stream.next().await {
        if let Ok(false) = msg.body().deserialize::<bool>() {
            RESUMED.notify_one();
        }
    }
}

fn strftime(fmt: &str, t: libc::time_t) -> String {
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    unsafe { libc::localtime_r(&t, &mut tm) };
    let mut buf = [0u8; 64];
    let cfmt = std::ffi::CString::new(fmt).unwrap_or_default();
    let n = unsafe { libc::strftime(buf.as_mut_ptr() as *mut libc::c_char, buf.len(), cfmt.as_ptr(), &tm) };
    String::from_utf8_lossy(&buf[..n]).into_owned()
}

fn now() -> libc::time_t {
    unsafe { libc::time(std::ptr::null_mut()) }
}

pub fn publish_clock() {
    let t = now();
    let twelve = crate::app::cfg().services.clock_format == "12h";
    let v = json!({
        "hm": strftime(if twelve { "%-I:%M" } else { "%H:%M" }, t),
        "ampm": if twelve { strftime("%p", t) } else { String::new() },
        "date": strftime("%a %-d", t),
        "long": strftime("%A, %B %-d", t),
    });
    app().hub.publish("clock", &v);
}

fn mmss(s: f64) -> String {
    let s = s.max(0.0) as u64;
    format!("{}:{:02}", s / 60, s % 60)
}

pub async fn run() {
    // Clock: re-arm on every minute boundary, and right after a resume.
    tokio::spawn(watch_resume());
    tokio::spawn(async {
        loop {
            publish_clock();
            let secs = now() % 60;
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_millis(((60 - secs) as u64) * 1000 + 50)) => {},
                _ = RESUMED.notified() => {},
            }
        }
    });
    // Media position.
    let mut rx = app().hub.subscribe("media").expect("media topic");
    loop {
        let m: Value = serde_json::from_str(&app().hub.get("media").unwrap_or_default()).unwrap_or_default();
        let playing = m["playing"] == true;
        let pos = m["pos"].as_f64().unwrap_or(0.0);
        let ts = m["ts"].as_f64().unwrap_or(0.0);
        let len = m["len"].as_f64().unwrap_or(0.0);
        let cur = if playing { (pos + now() as f64 - ts).min(len.max(pos)) } else { pos };
        app().hub.publish("mpos", &json!({ "pos": cur.round(), "len": len, "pt": mmss(cur), "lt": mmss(len) }));
        if playing && app().hub.demand("media-pos") {
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_secs(1)) => {},
                _ = rx.changed() => {},
                _ = WAKE.notified() => {},
            }
        } else {
            tokio::select! {
                r = rx.changed() => if r.is_err() { return },
                _ = WAKE.notified() => {},
            }
        }
    }
}
