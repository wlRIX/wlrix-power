// SPDX-License-Identifier: GPL-3.0-or-later
//! A stand-in UPower and power-profiles-daemon, for trying `wlrix-power` on a machine with no
//! battery -- or on one with a battery that should not have to be run down.
//!
//! It takes both names on whatever bus `DBUS_SYSTEM_BUS_ADDRESS` names, which is never the real
//! system bus: start a private one and point both this and the daemon at it.
//!
//! ```text
//! dbus-daemon --session --print-address --fork      # the address it prints is the fake system bus
//! DBUS_SYSTEM_BUS_ADDRESS=<address> cargo run --example fake_upower
//! DBUS_SYSTEM_BUS_ADDRESS=<address> wlrix-power
//! ```
//!
//! Then type lines on its standard input:
//!
//! ```text
//! battery <percent> <state> [warning]   state: charging, discharging, full, pending;
//!                                       warning: none, low, critical, action
//! none                                  no battery, as on a desktop
//! profile <name>                        what power-profiles-daemon says is active
//! no-profiles                           power-profiles-daemon goes away
//! ```

use std::collections::HashMap;
use std::io::BufRead;
use std::sync::{Arc, Mutex};

use zbus::zvariant::{OwnedValue, Value};

#[derive(Clone)]
struct Fake {
    present: bool,
    percentage: f64,
    state: u32,
    warning: u32,
    active: String,
}

impl Fake {
    fn icon(&self) -> String {
        if !self.present {
            return "battery-missing-symbolic".into();
        }
        let level = ((self.percentage / 10.0).round() as u32 * 10).min(100);
        match self.state {
            4 => "battery-level-100-charged-symbolic".into(),
            1 => format!("battery-level-{level}-charging-symbolic"),
            _ => format!("battery-level-{level}-symbolic"),
        }
    }
}

struct Device(Arc<Mutex<Fake>>);

#[zbus::interface(name = "org.freedesktop.UPower.Device")]
impl Device {
    #[zbus(property)]
    fn is_present(&self) -> bool {
        self.0.lock().unwrap().present
    }
    #[zbus(property, name = "Type")]
    fn kind(&self) -> u32 {
        if self.0.lock().unwrap().present { 2 } else { 0 }
    }
    #[zbus(property)]
    fn state(&self) -> u32 {
        self.0.lock().unwrap().state
    }
    #[zbus(property)]
    fn percentage(&self) -> f64 {
        self.0.lock().unwrap().percentage
    }
    #[zbus(property)]
    fn time_to_empty(&self) -> i64 {
        let fake = self.0.lock().unwrap();
        // About four hours from full.
        if fake.state == 2 {
            (fake.percentage * 144.0) as i64
        } else {
            0
        }
    }
    #[zbus(property)]
    fn time_to_full(&self) -> i64 {
        let fake = self.0.lock().unwrap();
        if fake.state == 1 {
            ((100.0 - fake.percentage) * 60.0) as i64
        } else {
            0
        }
    }
    #[zbus(property)]
    fn warning_level(&self) -> u32 {
        self.0.lock().unwrap().warning
    }
    #[zbus(property)]
    fn icon_name(&self) -> String {
        self.0.lock().unwrap().icon()
    }
}

struct PowerProfiles(Arc<Mutex<Fake>>);

#[zbus::interface(name = "org.freedesktop.UPower.PowerProfiles")]
impl PowerProfiles {
    #[zbus(property)]
    fn active_profile(&self) -> String {
        self.0.lock().unwrap().active.clone()
    }
    #[zbus(property)]
    fn set_active_profile(&mut self, profile: String) -> zbus::fdo::Result<()> {
        if !["power-saver", "balanced", "performance"].contains(&profile.as_str()) {
            return Err(zbus::fdo::Error::InvalidArgs(format!(
                "no such profile: {profile}"
            )));
        }
        eprintln!("fake_upower: asked for {profile}");
        self.0.lock().unwrap().active = profile;
        Ok(())
    }
    #[zbus(property)]
    fn profiles(&self) -> Vec<HashMap<String, OwnedValue>> {
        ["power-saver", "balanced", "performance"]
            .into_iter()
            .map(|name| {
                HashMap::from([(
                    "Profile".to_owned(),
                    OwnedValue::try_from(Value::from(name)).unwrap(),
                )])
            })
            .collect()
    }
}

/// Say that something changed. The daemon reads everything again on any signal, so which
/// properties are named does not matter.
fn changed(connection: &zbus::blocking::Connection, path: &str, interface: &str) {
    let none: HashMap<&str, Value> = HashMap::new();
    let invalidated: Vec<&str> = Vec::new();
    let _ = connection.emit_signal(
        None::<&str>,
        path,
        "org.freedesktop.DBus.Properties",
        "PropertiesChanged",
        &(interface, none, invalidated),
    );
}

fn main() -> zbus::Result<()> {
    let fake = Arc::new(Mutex::new(Fake {
        present: true,
        percentage: 73.0,
        state: 2,
        warning: 1,
        active: "balanced".into(),
    }));
    let device = "/org/freedesktop/UPower/devices/DisplayDevice";
    let profiles_path = "/org/freedesktop/UPower/PowerProfiles";
    let upower = zbus::blocking::connection::Builder::system()?
        .name("org.freedesktop.UPower")?
        .serve_at(device, Device(fake.clone()))?
        .build()?;
    let mut profiles = Some(
        zbus::blocking::connection::Builder::system()?
            .name("org.freedesktop.UPower.PowerProfiles")?
            .serve_at(profiles_path, PowerProfiles(fake.clone()))?
            .build()?,
    );
    eprintln!("fake_upower: ready");

    for line in std::io::stdin().lock().lines() {
        let line = line.map_err(|err| zbus::Error::Failure(err.to_string()))?;
        let words: Vec<&str> = line.split_whitespace().collect();
        let mut state = fake.lock().unwrap();
        match words.as_slice() {
            ["battery", percent, kind, rest @ ..] => {
                state.present = true;
                state.percentage = percent.parse().unwrap_or(50.0);
                state.state = match *kind {
                    "charging" => 1,
                    "discharging" => 2,
                    "full" => 4,
                    "pending" => 5,
                    _ => 0,
                };
                state.warning = match rest.first().copied() {
                    Some("low") => 3,
                    Some("critical") => 4,
                    Some("action") => 5,
                    _ => 1,
                };
            }
            ["none"] => state.present = false,
            ["profile", name] => state.active = (*name).to_owned(),
            ["no-profiles"] => profiles = None,
            [] => continue,
            _ => {
                eprintln!("fake_upower: did not understand {line:?}");
                continue;
            }
        }
        drop(state);
        changed(&upower, device, "org.freedesktop.UPower.Device");
        if let Some(profiles) = &profiles {
            changed(
                profiles,
                profiles_path,
                "org.freedesktop.UPower.PowerProfiles",
            );
        }
    }
    Ok(())
}
