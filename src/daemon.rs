// SPDX-License-Identifier: GPL-3.0-or-later
//! The main loop.
//!
//! Five sources on one calloop loop: "UPower said something" from [`crate::upower::watch`], "a
//! tray appeared" from [`crate::sni::watch_for_tray`], what was chosen in the menu from
//! [`crate::dbusmenu`], and the reload and quit pings from [`crate::signals`]. The loop owns the
//! last snapshot, the config and the warning on screen, and is the only thing that changes what
//! the item and its menu show.
//!
//! # Warnings
//!
//! UPower works out the warning level itself -- from `PercentageLow`, `PercentageCritical` and
//! `PercentageAction` in `/etc/UPower/UPower.conf`, or from time left when `UsePercentageForPolicy`
//! is off -- so the thresholds are the system's, and the same ones that make UPower suspend or
//! power off at the action level. This daemon only tells the person, through the session's
//! notification daemon (`wlrix-osd`): once per level, each replacing the last, and taken down
//! when the machine is plugged in. See [`crate::indicator::decide`].
//!
//! Low is a normal notification; critical and action are `urgency` 2, which `wlrix-osd` leaves on
//! screen until it is dismissed or the warning is taken down from here.
//!
//! # Letting a burst settle
//!
//! A battery reports its level, its rate and its time left as separate properties, and the display
//! device follows each one with its own `PropertiesChanged`. A signal only *arms* a short timer,
//! and the whole state is read again when it fires -- the same as `wlrix-network`.

use std::collections::HashMap;
use std::time::Duration;

use calloop::EventLoop;
use calloop::timer::{TimeoutAction, Timer};

use crate::config::Config;
use crate::dbusmenu::MenuEvent;
use crate::i18n::tr;
use crate::indicator::{self, Alert, Decision, Status, View};
use crate::menu::{self, Action};
use crate::sni::{self, Published};
use crate::upower::{self, Snapshot, Warning};

/// How long after the first of a burst of UPower signals the state is read.
const SETTLE: Duration = Duration::from_millis(250);

const NOTIFICATIONS: &str = "org.freedesktop.Notifications";
const NOTIFICATIONS_PATH: &str = "/org/freedesktop/Notifications";

struct Daemon {
    config: Config,
    system: zbus::blocking::Connection,
    published: Published,
    /// What UPower said last. `None` when it did not answer.
    snapshot: Option<Snapshot>,
    /// Whether a read is already on its way.
    settling: bool,
    /// What the log last said was showing, so a line is written when that changes and not for
    /// every read.
    logged: Option<(String, Status)>,
    /// The level last warned about, and the warning's notification id while it is up.
    warned: Warning,
    notification: Option<u32>,
    exit: bool,
}

impl Daemon {
    /// Read UPower again and show the result.
    fn refresh(&mut self) {
        self.snapshot = match upower::read(&self.system) {
            Ok(snapshot) => Some(snapshot),
            Err(err) => {
                // Reported once per change of answer rather than once per read.
                if self.snapshot.is_some() {
                    eprintln!("wlrix-power: UPower is not answering: {err}");
                }
                None
            }
        };
        self.show();
        self.warn();
    }

    fn show(&mut self) {
        let view = self.view();
        self.log(&view);
        self.published.show(view);
        self.published.set_menu(menu::build(self.snapshot.as_ref()));
    }

    /// Put up, replace or take down the low-battery warning.
    fn warn(&mut self) {
        if !self.config.warnings() {
            self.clear();
            return;
        }
        let battery = self
            .snapshot
            .as_ref()
            .and_then(|snapshot| snapshot.battery.as_ref());
        match indicator::decide(self.warned, battery) {
            Decision::Nothing => {}
            Decision::Clear => self.clear(),
            Decision::Warn(level, alert) => {
                eprintln!("wlrix-power: {}: {}", alert.summary, alert.body);
                self.warned = level;
                let icon = self.view().icon;
                self.notification = self.notify(&alert, &icon, self.notification);
            }
        }
    }

    /// Take the warning down, and re-arm every level.
    fn clear(&mut self) {
        self.warned = Warning::None;
        if let Some(id) = self.notification.take()
            && let Err(err) = self.published.connection().call_method(
                Some(NOTIFICATIONS),
                NOTIFICATIONS_PATH,
                Some(NOTIFICATIONS),
                "CloseNotification",
                &(id,),
            )
        {
            eprintln!("wlrix-power: could not take the battery warning down: {err}");
        }
    }

    /// Post a notification, replacing `replaces` if it is still up. Its id, when it was posted.
    ///
    /// Not delivered is not reported beyond the log: the session's notification daemon is the
    /// only place it could be shown.
    fn notify(&self, alert: &Alert, icon: &str, replaces: Option<u32>) -> Option<u32> {
        let mut hints: HashMap<&str, zbus::zvariant::Value> = HashMap::new();
        if alert.critical {
            hints.insert("urgency", zbus::zvariant::Value::U8(2));
        }
        let actions: Vec<&str> = Vec::new();
        let reply = self.published.connection().call_method(
            Some(NOTIFICATIONS),
            NOTIFICATIONS_PATH,
            Some(NOTIFICATIONS),
            "Notify",
            &(
                tr("Battery", "バッテリー"),
                replaces.unwrap_or(0),
                icon,
                alert.summary.as_str(),
                alert.body.as_str(),
                actions,
                hints,
                -1i32,
            ),
        );
        match reply.and_then(|reply| reply.body().deserialize::<u32>()) {
            Ok(id) => Some(id),
            Err(err) => {
                eprintln!(
                    "wlrix-power: could not post a notification ({err}): {}",
                    alert.summary
                );
                None
            }
        }
    }

    fn on_menu(&mut self, event: MenuEvent) {
        let MenuEvent::Chosen(action) = event else {
            // Nothing to fetch before the menu opens: it is current already.
            return;
        };
        match &action {
            Action::Profile(profile) => {
                let active = self
                    .snapshot
                    .as_ref()
                    .and_then(|snapshot| snapshot.profiles.as_ref())
                    .map(|profiles| profiles.active.as_str());
                if active == Some(profile.as_str()) {
                    return;
                }
                // What follows arrives as a PropertiesChanged like any other change.
                if let Err(err) = upower::set_profile(&self.system, profile) {
                    eprintln!("wlrix-power: {action:?} failed: {err}");
                    self.post(
                        tr(
                            "The power mode was not changed",
                            "電源モードを変更できませんでした",
                        ),
                        &refusal(&err),
                    );
                }
            }
        }
    }

    /// A plain notification, for a refusal.
    fn post(&self, summary: &str, body: &str) {
        let alert = Alert {
            summary: summary.to_owned(),
            body: body.to_owned(),
            critical: false,
        };
        self.notify(&alert, "dialog-error-symbolic", None);
    }

    /// One line in the session log when what is shown changes.
    fn log(&mut self, view: &View) {
        let now = (view.title.clone(), view.status);
        if self.logged.as_ref() != Some(&now) {
            eprintln!("wlrix-power: {} ({})", view.title, view.status.as_str());
            self.logged = Some(now);
        }
    }

    fn view(&self) -> View {
        indicator::view(self.snapshot.as_ref(), self.config.indicator)
    }
}

/// How long any call this daemon makes may take before it is given up on, so a service that
/// never answers cannot stop the loop -- the same as `wlrix-network`.
pub const CALL_TIMEOUT: Duration = Duration::from_secs(10);

fn system_bus() -> zbus::Result<zbus::blocking::Connection> {
    zbus::blocking::connection::Builder::system()?
        .method_timeout(CALL_TIMEOUT)
        .build()
}

/// Run until stopped.
pub fn run() -> Result<(), String> {
    let config = Config::load();
    let system =
        system_bus().map_err(|err| format!("no system bus, so no UPower to ask: {err}"))?;

    let snapshot = upower::read(&system).ok();
    if snapshot.is_none() {
        // Not fatal: UPower is D-Bus activated, and its name appearing is one of the signals
        // watched for.
        eprintln!("wlrix-power: UPower is not on the system bus yet");
    }
    let first = indicator::view(snapshot.as_ref(), config.indicator);
    let root = menu::build(snapshot.as_ref());
    let (menu_events, menu_channel) = calloop::channel::channel();
    let published = sni::publish(first.clone(), root, menu_events)?;

    let mut event_loop: EventLoop<Daemon> =
        EventLoop::try_new().map_err(|err| format!("could not create the event loop: {err}"))?;
    let handle = event_loop.handle();

    let (changed, changes) = calloop::channel::channel();
    upower::watch(system.clone(), changed)?;
    let settle_handle = handle.clone();
    handle
        .insert_source(changes, move |event, _, daemon: &mut Daemon| {
            if !matches!(event, calloop::channel::Event::Msg(())) || daemon.settling {
                return;
            }
            daemon.settling = true;
            let armed = settle_handle.insert_source(
                Timer::from_duration(SETTLE),
                |_, _, daemon: &mut Daemon| {
                    daemon.settling = false;
                    daemon.refresh();
                    TimeoutAction::Drop
                },
            );
            if let Err(err) = armed {
                eprintln!("wlrix-power: could not arm the settle timer: {err}");
                daemon.settling = false;
                daemon.refresh();
            }
        })
        .map_err(|err| format!("could not listen to UPower: {err}"))?;

    handle
        .insert_source(menu_channel, |event, _, daemon: &mut Daemon| {
            if let calloop::channel::Event::Msg(event) = event {
                daemon.on_menu(event);
            }
        })
        .map_err(|err| format!("could not listen to the menu: {err}"))?;

    let (appeared, appearances) = calloop::channel::channel();
    sni::watch_for_tray(&published, appeared)?;
    handle
        .insert_source(appearances, |event, _, daemon: &mut Daemon| {
            if let calloop::channel::Event::Msg(()) = event {
                daemon.published.register();
            }
        })
        .map_err(|err| format!("could not listen for the tray: {err}"))?;

    let (reload_ping, reload_source) = calloop::ping::make_ping()
        .map_err(|err| format!("could not create the reload ping: {err}"))?;
    handle
        .insert_source(reload_source, |_, _, daemon: &mut Daemon| {
            daemon.config = Config::load();
            daemon.show();
            daemon.warn();
        })
        .map_err(|err| format!("could not watch for reloads: {err}"))?;
    crate::signals::forward_reload_to_loop(reload_ping);

    let (quit_ping, quit_source) = calloop::ping::make_ping()
        .map_err(|err| format!("could not create the quit ping: {err}"))?;
    handle
        .insert_source(quit_source, |_, _, daemon: &mut Daemon| daemon.exit = true)
        .map_err(|err| format!("could not watch for a stop: {err}"))?;
    crate::signals::forward_to_loop(quit_ping);

    // Registered last, once everything that keeps the item current is in place: the tray reads
    // the item the moment it is registered.
    published.register();

    let mut daemon = Daemon {
        config,
        system,
        published,
        snapshot,
        settling: false,
        logged: None,
        warned: Warning::None,
        notification: None,
        exit: false,
    };
    daemon.log(&first);
    // A session started on a low battery is warned straight away.
    daemon.warn();
    let _pidfile = crate::pidfile::write();

    while !daemon.exit {
        event_loop
            .dispatch(None, &mut daemon)
            .map_err(|err| format!("event loop failed: {err}"))?;
    }
    // A warning left up would outlive the daemon that can take it down.
    daemon.clear();
    Ok(())
}

/// Read UPower once and say what the icon would show, for `--status`.
pub fn status() -> Result<(), String> {
    let config = Config::load();
    let system = system_bus().map_err(|err| format!("no system bus: {err}"))?;
    let snapshot = upower::read(&system).ok();
    let view = indicator::view(snapshot.as_ref(), config.indicator);
    println!("status:  {}", view.status.as_str());
    println!("icon:    {}", view.icon);
    println!("title:   {}", view.title);
    if !view.body.is_empty() {
        println!("         {}", view.body);
    }
    match &snapshot {
        None => println!("UPower did not answer"),
        Some(snapshot) => {
            if let Some(battery) = &snapshot.battery {
                println!(
                    "state:   {:?}, warning level {:?}",
                    battery.state, battery.warning
                );
            }
            match &snapshot.profiles {
                Some(profiles) => println!(
                    "mode:    {} (of {})",
                    profiles.active,
                    profiles.available.join(", ")
                ),
                None => println!("mode:    power-profiles-daemon is not running"),
            }
        }
    }
    Ok(())
}

/// What the service said when it refused, without the D-Bus error name in front.
fn refusal(err: &zbus::Error) -> String {
    match err {
        zbus::Error::MethodError(_, Some(description), _) => description.clone(),
        other => other.to_string(),
    }
}
