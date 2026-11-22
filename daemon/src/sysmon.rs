//! System load. Two modes, both sleeping on a Notify when nothing needs them:
//! * light: CPU + memory percentages for the bar pills (`load` topic), every
//!   `bar.sysInterval` ms — two small /proc reads;
//! * full: also disk usage and memory sizes (`sys` topic), only while the
//!   control center is open.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::app::app;
use crate::util::{human_bytes, read_trim};

#[derive(Default)]
struct Inner {
    last_cpu: Option<(u64, u64)>,
    storage: Value,
    storage_at: Option<Instant>,
}

#[derive(Default)]
pub struct SysMon {
    inner: Mutex<Inner>,
    wake: tokio::sync::Notify,
}

fn cpu_times() -> Option<(u64, u64)> {
    let s = std::fs::read_to_string("/proc/stat").ok()?;
    let line = s.lines().next()?;
    let v: Vec<u64> = line.split_whitespace().skip(1).filter_map(|x| x.parse().ok()).collect();
    if v.len() < 5 {
        return None;
    }
    let idle = v[3] + v.get(4).copied().unwrap_or(0);
    let total: u64 = v.iter().take(8).sum();
    Some((idle, total))
}

fn meminfo() -> (u64, u64) {
    let s = std::fs::read_to_string("/proc/meminfo").unwrap_or_default();
    let mut total = 0;
    let mut avail = 0;
    for l in s.lines() {
        let mut it = l.split_whitespace();
        match (it.next(), it.next().and_then(|v| v.parse::<u64>().ok())) {
            (Some("MemTotal:"), Some(v)) => total = v * 1024,
            (Some("MemAvailable:"), Some(v)) => avail = v * 1024,
            _ => {}
        }
    }
    (total.saturating_sub(avail), total)
}

fn storage() -> Value {
    let bin = crate::app::cfg().services.data_units != "decimal";
    let mounts = std::fs::read_to_string("/proc/mounts").unwrap_or_default();
    let fs_ok = ["ext4", "ext3", "btrfs", "xfs", "f2fs", "zfs", "bcachefs", "ntfs3", "exfat", "vfat"];
    let mut seen_dev = std::collections::HashSet::new();
    let mut out = vec![];
    for l in mounts.lines() {
        let f: Vec<&str> = l.split_whitespace().collect();
        if f.len() < 3 || !fs_ok.contains(&f[2]) {
            continue;
        }
        let (dev, mnt) = (f[0], f[1].replace("\\040", " "));
        if mnt.starts_with("/boot") || mnt.starts_with("/nix/store") || mnt.starts_with("/efi") {
            continue;
        }
        // ZFS: group per pool; others per device.
        let key = if f[2] == "zfs" { dev.split('/').next().unwrap_or(dev).to_string() } else { dev.to_string() };
        if !seen_dev.insert(key.clone()) {
            continue;
        }
        let c = std::ffi::CString::new(mnt.clone()).unwrap_or_default();
        let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
        if unsafe { libc::statvfs(c.as_ptr(), &mut st) } != 0 {
            continue;
        }
        // Same numbers as df (and DMS): used = blocks - free, percentage
        // of what non-root users can reach (used + available).
        let fr = st.f_frsize as f64;
        let total = st.f_blocks as f64 * fr;
        let used = (st.f_blocks.saturating_sub(st.f_bfree)) as f64 * fr;
        let reachable = used + st.f_bavail as f64 * fr;
        if total < 1e9 || reachable <= 0.0 {
            continue;
        }
        out.push(json!({
            "mount": mnt,
            "used": human_bytes(used, bin), "total": human_bytes(total, bin), "pct": (used / reachable * 100.0).round() as i64,
        }));
        if out.len() >= 4 {
            break;
        }
    }
    Value::Array(out)
}

impl SysMon {
    pub fn wake(&self) {
        self.wake.notify_one();
    }

    /// CPU busy fraction since the previous call.
    fn cpu(&self) -> f64 {
        let mut i = self.inner.lock().unwrap();
        match (cpu_times(), i.last_cpu) {
            (Some((idle, total)), Some((pi, pt))) if total > pt => {
                i.last_cpu = Some((idle, total));
                1.0 - (idle.saturating_sub(pi)) as f64 / (total - pt) as f64
            }
            (Some(c), _) => {
                i.last_cpu = Some(c);
                0.0
            }
            _ => 0.0,
        }
    }

    fn sample(&self, full: bool) {
        let cpu = self.cpu();
        let mem = meminfo();
        let pct = if mem.1 > 0 { (mem.0 as f64 / mem.1 as f64 * 100.0).round() as i64 } else { 0 };
        app().hub.publish("load", &json!({ "cpu": (cpu * 100.0).round() as i64, "mem": pct }));
        if !full {
            return;
        }
        let bin = crate::app::cfg().services.data_units != "decimal";
        let storage = {
            let mut i = self.inner.lock().unwrap();
            // Disk usage changes slowly: statvfs at most every 30 s.
            if i.storage_at.map(|t| t.elapsed() > Duration::from_secs(30)).unwrap_or(true) {
                i.storage = storage();
                i.storage_at = Some(Instant::now());
            }
            i.storage.clone()
        };
        app().hub.publish(
            "sys",
            &json!({
                "storage": storage,
                "memUsed": human_bytes(mem.0 as f64, bin),
                "memTotal": human_bytes(mem.1 as f64, bin),
                "uptime": uptime(),
            }),
        );
    }
}

pub fn uptime() -> String {
    let secs = read_trim("/proc/uptime").and_then(|s| s.split_whitespace().next().and_then(|x| x.parse::<f64>().ok())).unwrap_or(0.0)
        as u64;
    let (d, h, m) = (secs / 86400, (secs % 86400) / 3600, (secs % 3600) / 60);
    let mut parts = vec![];
    if d > 0 {
        parts.push(format!("{d} day{}", if d == 1 { "" } else { "s" }));
    }
    if h > 0 {
        parts.push(format!("{h} hour{}", if h == 1 { "" } else { "s" }));
    }
    if d == 0 {
        parts.push(format!("{m} minute{}", if m == 1 { "" } else { "s" }));
    }
    format!("up {}", parts.join(", "))
}

pub async fn run() {
    let sm = &app().sys;
    loop {
        let full = app().hub.demand("sys");
        let light_ms = {
            let c = crate::app::cfg();
            let used = c.bar.left.iter().chain(&c.bar.center).chain(&c.bar.right).any(|w| w == "cpuUsage" || w == "memUsage");
            if used { c.bar.sys_interval } else { 0 }
        };
        if !full && light_ms == 0 {
            sm.wake.notified().await;
            continue;
        }
        sm.sample(full);
        // With the control center open the bar pills keep their own pace.
        let res = crate::app::cfg().dashboard.resource_update_interval.max(250);
        let ms = match (full, light_ms) {
            (true, 0) => res,
            (true, l) => res.min(l.max(500)),
            (false, l) => l.max(500),
        };
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_millis(ms)) => {},
            _ = sm.wake.notified() => {},
        }
    }
}
