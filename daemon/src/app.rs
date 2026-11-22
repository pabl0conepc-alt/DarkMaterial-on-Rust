//! Daemon wiring: one global `App`, a single-threaded tokio runtime, and all
//! services started as lightweight tasks. Everything is event driven; the only
//! periodic timers are cheap sysfs reads (battery every 15s) and the resource
//! monitor, which only ticks while a panel that shows it is open.

use std::sync::{Mutex, OnceLock, RwLock};

use crate::config::Config;
use crate::hub::Hub;

pub struct Misc {
    pub dnd: bool,
    pub idle_inhibit: bool,
    pub gamemode: bool,
    pub idle_inhibitor: Option<std::process::Child>,
    pub night: bool,
    pub night_child: Option<std::process::Child>,
}

pub struct App {
    pub hub: Hub,
    pub cfg: RwLock<std::sync::Arc<Config>>,
    pub eww: crate::eww::Eww,
    pub ui: crate::ui::Ui,
    pub audio: crate::audio::Audio,
    pub bright: crate::brightness::Brightness,
    pub bat: crate::battery::Battery,
    pub net: crate::network::Network,
    pub bt: crate::bluetooth::Bluetooth,
    pub media: crate::media::Media,
    pub notifs: crate::notifd::Notifd,
    pub sys: crate::sysmon::SysMon,
    pub launcher: crate::launcher::Launcher,
    pub prefs: crate::prefs::Prefs,
    pub theme: crate::theme::Theme,
    pub wm: crate::wm::Wm,
    pub info: crate::info::Info,
    pub misc: Mutex<Misc>,
    pub session_bus: OnceLock<zbus::Connection>,
    pub system_bus: OnceLock<zbus::Connection>,
}

static APP: OnceLock<App> = OnceLock::new();

pub fn app() -> &'static App {
    APP.get().expect("app not initialised")
}

/// Cheap snapshot of the configuration (an Arc clone, safe across awaits).
pub fn cfg() -> std::sync::Arc<Config> {
    app().cfg.read().unwrap().clone()
}

impl App {
    /// Rebuild the `status` topic (everything the bar's status icons, OSD and
    /// quick toggles need) from the individual services.
    pub fn publish_status(&self) {
        let misc = self.misc.lock().unwrap();
        let v = serde_json::json!({
            "audio": self.audio.json(),
            "bright": self.bright.json(),
            "bat": self.bat.json(),
            "net": self.net.json(),
            "bt": self.bt.json(),
            "kb": self.wm.kb_json(),
            "dnd": misc.dnd,
            "idleInhibit": misc.idle_inhibit,
            "gamemode": misc.gamemode,
            "night": misc.night,
        });
        drop(misc);
        self.hub.publish("status", &v);
    }

    pub fn ui_active_height(&self) -> i32 {
        self.ui.active_height_pub()
    }
}

pub fn run_daemon() -> anyhow::Result<()> {
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build()?;
    rt.block_on(async_main())
}

async fn async_main() -> anyhow::Result<()> {
    let rt_dir = crate::util::runtime_dir();
    std::fs::create_dir_all(&rt_dir)?;
    for d in [crate::util::state_dir(), crate::util::cache_dir()] {
        let _ = std::fs::create_dir_all(d);
    }

    let cfg = Config::load();
    let app = App {
        hub: Hub::new(),
        eww: crate::eww::Eww::new(),
        ui: crate::ui::Ui::default(),
        audio: Default::default(),
        bright: Default::default(),
        bat: Default::default(),
        net: Default::default(),
        bt: Default::default(),
        media: Default::default(),
        notifs: Default::default(),
        sys: Default::default(),
        launcher: Default::default(),
        prefs: Default::default(),
        theme: Default::default(),
        wm: Default::default(),
        info: Default::default(),
        misc: Mutex::new(Misc { dnd: false, idle_inhibit: false, gamemode: false, idle_inhibitor: None, night: false, night_child: None }),
        session_bus: OnceLock::new(),
        system_bus: OnceLock::new(),
        cfg: RwLock::new(std::sync::Arc::new(cfg)),
    };
    if APP.set(app).is_err() {
        anyhow::bail!("daemon initialised twice");
    }
    let app = self::app();

    // Socket first, so `dmr listen` clients spawned by EWW connect at once.
    let listener = crate::server::bind()?;

    // D-Bus connections (both optional: the shell degrades gracefully).
    match zbus::Connection::session().await {
        Ok(c) => {
            let _ = app.session_bus.set(c);
        }
        Err(e) => eprintln!("dmr: no session bus: {e}"),
    }
    match zbus::Connection::system().await {
        Ok(c) => {
            let _ = app.system_bus.set(c);
        }
        Err(e) => eprintln!("dmr: no system bus: {e}"),
    }

    // Theme must exist before EWW compiles its SCSS.
    app.theme.init().await;
    app.ui.publish();
    app.publish_status();
    crate::settings::publish();

    tokio::spawn(crate::server::serve(listener));
    tokio::spawn(crate::wm::run());
    tokio::spawn(crate::audio::run());
    tokio::spawn(crate::brightness::run());
    tokio::spawn(crate::battery::run());
    tokio::spawn(crate::network::run());
    tokio::spawn(crate::bluetooth::run());
    tokio::spawn(crate::media::run());
    tokio::spawn(crate::notifd::run());
    tokio::spawn(crate::sysmon::run());
    tokio::spawn(crate::info::run());
    tokio::spawn(crate::clock::run());
    tokio::spawn(crate::eww::restart_on_crash());
    tokio::spawn(crate::prefs::loader());
    tokio::spawn(async { app.launcher.preload().await });

    // EWW last: it immediately spawns its listeners which need the socket.
    app.eww.start().await;
    app.ui.open_static().await;

    // Run until asked to quit or signalled.
    let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    let mut int = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())?;
    tokio::select! {
        _ = term.recv() => {},
        _ = int.recv() => {},
        _ = app.ui.quit_requested() => {},
    }
    shutdown().await;
    Ok(())
}

pub async fn shutdown() {
    let app = app();
    app.notifs.save();
    app.eww.kill().await;
    {
        let mut m = app.misc.lock().unwrap();
        for c in [m.idle_inhibitor.take(), m.night_child.take()].into_iter().flatten() {
            let mut c = c;
            let _ = c.kill();
        }
    }
    let _ = std::fs::remove_file(crate::util::socket_path());
}
