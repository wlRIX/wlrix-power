// SPDX-License-Identifier: GPL-3.0-or-later
//! UPower and power-profiles-daemon, read over the system bus.
//!
//! # The display device
//!
//! UPower publishes one device per battery, mouse, UPS and phone it knows of, and one more that
//! is none of them: `/org/freedesktop/UPower/devices/DisplayDevice`, the summary a desktop is
//! meant to show. On a laptop with two batteries it is their combined charge and the time left
//! on both; on a desktop it says `IsPresent = false`. It even carries the icon name to show,
//! worked out from the level and the charging state with the same buckets every desktop uses, so
//! the icon here agrees with any other on the machine. Reading it and nothing else is the whole
//! of what an indicator needs.
//!
//! # Read it all again
//!
//! As in `wlrix-network`: any signal from either daemon means read the state again, after the
//! main loop lets a burst settle. Two `GetAll` calls; there is no state worth patching.
//!
//! # Power modes
//!
//! power-profiles-daemon's `performance`, `balanced` and `power-saver`, the same three every
//! desktop offers. It is a daemon of its own, with its own bus name, though its object sits under
//! UPower's path: `org.freedesktop.UPower.PowerProfiles` at `/org/freedesktop/UPower/
//! PowerProfiles` (`net.hadess.PowerProfiles` is the older spelling, kept for old clients).
//! Absent, the menu simply has no power modes.

use std::collections::HashMap;

use zbus::MatchRule;
use zbus::message::Type;
use zbus::zvariant::{OwnedValue, Value};

const UPOWER: &str = "org.freedesktop.UPower";
const UPOWER_ROOT: &str = "/org/freedesktop/UPower";
const DISPLAY_DEVICE: &str = "/org/freedesktop/UPower/devices/DisplayDevice";
const DEVICE: &str = "org.freedesktop.UPower.Device";
/// power-profiles-daemon's bus name, and the interface it serves -- both spelled the same.
const PROFILES: &str = "org.freedesktop.UPower.PowerProfiles";
const PROFILES_PATH: &str = "/org/freedesktop/UPower/PowerProfiles";
const PROPERTIES: &str = "org.freedesktop.DBus.Properties";
const DBUS: &str = "org.freedesktop.DBus";

/// `UP_DEVICE_KIND_BATTERY`: the only kind the display device is when there is one to show.
const KIND_BATTERY: u32 = 2;

/// What the battery is doing, from `UpDeviceState`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum State {
    #[default]
    Unknown,
    Charging,
    Discharging,
    Empty,
    FullyCharged,
    /// Plugged in, and the charger is holding off -- a charge limit, or a battery too warm.
    PendingCharge,
    PendingDischarge,
}

impl State {
    pub fn from_code(code: u32) -> Self {
        match code {
            1 => Self::Charging,
            2 => Self::Discharging,
            3 => Self::Empty,
            4 => Self::FullyCharged,
            5 => Self::PendingCharge,
            6 => Self::PendingDischarge,
            _ => Self::Unknown,
        }
    }

    /// Whether the machine is running from the battery rather than the wall.
    pub fn on_battery(self) -> bool {
        matches!(
            self,
            Self::Discharging | Self::Empty | Self::PendingDischarge
        )
    }
}

/// How worried UPower is, from `UpDeviceLevel`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum Warning {
    #[default]
    None,
    /// Low: about ten minutes, or ten percent, left.
    Low,
    /// Critical: a few minutes left.
    Critical,
    /// Action: UPower is about to do its `CriticalPowerAction` -- suspend, hibernate or power off.
    Action,
}

impl Warning {
    pub fn from_code(code: u32) -> Self {
        match code {
            3 => Self::Low,
            4 => Self::Critical,
            5 => Self::Action,
            // 1 is none, 2 is "discharging", which UPower sets only for a UPS on battery.
            _ => Self::None,
        }
    }
}

/// The battery, as the display device describes it.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Battery {
    pub state: State,
    pub percentage: f64,
    /// Seconds. Zero when UPower has not estimated it yet, which is normal for a minute or so
    /// after the state changes.
    pub time_to_empty: i64,
    pub time_to_full: i64,
    pub warning: Warning,
    /// UPower's own choice of icon, `battery-level-70-charging-symbolic` and the like.
    pub icon: String,
}

/// Everything the item is drawn from.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Snapshot {
    /// `None` when there is no battery to describe.
    pub battery: Option<Battery>,
    /// `None` when power-profiles-daemon is not running.
    pub profiles: Option<Profiles>,
}

/// The power modes on offer, and which is in force.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Profiles {
    pub available: Vec<String>,
    pub active: String,
}

/// Read both daemons. `Err` only when UPower itself does not answer; power-profiles-daemon
/// missing is an ordinary `profiles: None`.
pub fn read(connection: &zbus::blocking::Connection) -> zbus::Result<Snapshot> {
    let device = get_all(connection, UPOWER, DISPLAY_DEVICE, DEVICE)?;
    Ok(Snapshot {
        battery: battery(&device),
        profiles: get_all(connection, PROFILES, PROFILES_PATH, PROFILES)
            .ok()
            .and_then(|properties| profiles(&properties)),
    })
}

/// The display device's properties, read into a battery -- or `None` when there is none.
fn battery(properties: &HashMap<String, OwnedValue>) -> Option<Battery> {
    let present: bool = take(properties, "IsPresent").unwrap_or(false);
    let kind: u32 = take(properties, "Type").unwrap_or(0);
    if !present || kind != KIND_BATTERY {
        return None;
    }
    Some(Battery {
        state: State::from_code(take(properties, "State").unwrap_or(0)),
        percentage: take::<f64>(properties, "Percentage")
            .unwrap_or(0.0)
            .clamp(0.0, 100.0),
        time_to_empty: take(properties, "TimeToEmpty").unwrap_or(0),
        time_to_full: take(properties, "TimeToFull").unwrap_or(0),
        warning: Warning::from_code(take(properties, "WarningLevel").unwrap_or(0)),
        icon: take(properties, "IconName").unwrap_or_default(),
    })
}

/// power-profiles-daemon's `Profiles` and `ActiveProfile`.
fn profiles(properties: &HashMap<String, OwnedValue>) -> Option<Profiles> {
    let list: Vec<HashMap<String, OwnedValue>> = take(properties, "Profiles")?;
    let available: Vec<String> = list
        .iter()
        .filter_map(|profile| take::<String>(profile, "Profile"))
        .collect();
    if available.is_empty() {
        return None;
    }
    Some(Profiles {
        available,
        active: take(properties, "ActiveProfile").unwrap_or_default(),
    })
}

/// Switch the power mode. power-profiles-daemon's polkit policy lets the active session do it.
pub fn set_profile(connection: &zbus::blocking::Connection, profile: &str) -> zbus::Result<()> {
    connection.call_method(
        Some(PROFILES),
        PROFILES_PATH,
        Some(PROPERTIES),
        "Set",
        &(PROFILES, "ActiveProfile", Value::from(profile)),
    )?;
    Ok(())
}

fn take<T>(map: &HashMap<String, OwnedValue>, key: &str) -> Option<T>
where
    T: TryFrom<OwnedValue>,
{
    map.get(key)?.try_clone().ok()?.try_into().ok()
}

fn get_all(
    connection: &zbus::blocking::Connection,
    destination: &str,
    path: &str,
    interface: &str,
) -> zbus::Result<HashMap<String, OwnedValue>> {
    let reply = connection.call_method(
        Some(destination),
        path,
        Some(PROPERTIES),
        "GetAll",
        &(interface,),
    )?;
    reply.body().deserialize()
}

/// Ask for every signal that can change what is shown: property changes under UPower's root --
/// the display device's, and power-profiles-daemon's, whose object lives under the same path
/// though it is a different process with a name of its own -- and both names coming and going.
pub fn subscribe(connection: &zbus::blocking::Connection) -> zbus::Result<()> {
    let bus = zbus::blocking::fdo::DBusProxy::new(connection)?;
    bus.add_match_rule(
        MatchRule::builder()
            .msg_type(Type::Signal)
            .interface(PROPERTIES)?
            .member("PropertiesChanged")?
            .path_namespace(UPOWER_ROOT)?
            .build(),
    )?;
    for name in [UPOWER, PROFILES] {
        bus.add_match_rule(
            MatchRule::builder()
                .msg_type(Type::Signal)
                .sender(DBUS)?
                .interface(DBUS)?
                .member("NameOwnerChanged")?
                .add_arg(name)?
                .build(),
        )?;
    }
    Ok(())
}

/// Start the thread that turns those signals into "something changed" on the loop.
pub fn watch(
    connection: zbus::blocking::Connection,
    changed: calloop::channel::Sender<()>,
) -> Result<(), String> {
    subscribe(&connection).map_err(|err| format!("could not watch UPower: {err}"))?;
    std::thread::Builder::new()
        .name("upower".into())
        .spawn(move || {
            for message in zbus::blocking::MessageIterator::from(&connection) {
                let Ok(message) = message else { continue };
                // The reads share this connection, so their replies come through here too.
                if message.message_type() != Type::Signal {
                    continue;
                }
                if changed.send(()).is_err() {
                    return;
                }
            }
        })
        .map_err(|err| format!("could not start the UPower thread: {err}"))?;
    Ok(())
}

#[cfg(test)]
pub(crate) fn owned<'a>(value: impl Into<Value<'a>>) -> OwnedValue {
    value.into().try_into().unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device(pairs: Vec<(&str, OwnedValue)>) -> HashMap<String, OwnedValue> {
        pairs.into_iter().map(|(k, v)| (k.to_owned(), v)).collect()
    }

    #[test]
    fn a_desktop_has_no_battery() {
        // What UPower says on this machine: present false, type unknown, icon battery-missing.
        let desktop = device(vec![
            ("IsPresent", owned(false)),
            ("Type", owned(0u32)),
            ("IconName", owned("battery-missing-symbolic")),
        ]);
        assert_eq!(battery(&desktop), None);
        // And a UPS, or a mouse that somehow became the display device, is not a battery either.
        let ups = device(vec![("IsPresent", owned(true)), ("Type", owned(3u32))]);
        assert_eq!(battery(&ups), None);
    }

    #[test]
    fn a_laptop_battery_is_read() {
        let laptop = device(vec![
            ("IsPresent", owned(true)),
            ("Type", owned(2u32)),
            ("State", owned(2u32)),
            ("Percentage", owned(73.4f64)),
            ("TimeToEmpty", owned(7800i64)),
            ("TimeToFull", owned(0i64)),
            ("WarningLevel", owned(1u32)),
            ("IconName", owned("battery-level-70-symbolic")),
        ]);
        let read = battery(&laptop).unwrap();
        assert_eq!(read.state, State::Discharging);
        assert!(read.state.on_battery());
        assert_eq!(read.time_to_empty, 7800);
        assert_eq!(read.warning, Warning::None);
        assert_eq!(read.icon, "battery-level-70-symbolic");
    }

    #[test]
    fn warning_levels_are_upowers() {
        assert_eq!(Warning::from_code(1), Warning::None);
        assert_eq!(
            Warning::from_code(2),
            Warning::None,
            "a UPS on battery is not low"
        );
        assert_eq!(Warning::from_code(3), Warning::Low);
        assert_eq!(Warning::from_code(4), Warning::Critical);
        assert_eq!(Warning::from_code(5), Warning::Action);
        assert!(Warning::Critical > Warning::Low);
    }

    #[test]
    fn power_modes_are_read_from_the_profile_list() {
        let profile = |name: &str| {
            let mut map = HashMap::new();
            map.insert("Profile".to_owned(), owned(name));
            map.insert("Driver".to_owned(), owned("platform_profile"));
            map
        };
        let properties = device(vec![
            (
                "Profiles",
                OwnedValue::try_from(Value::from(vec![
                    profile("power-saver"),
                    profile("balanced"),
                    profile("performance"),
                ]))
                .unwrap(),
            ),
            ("ActiveProfile", owned("balanced")),
        ]);
        let read = profiles(&properties).unwrap();
        assert_eq!(read.available, ["power-saver", "balanced", "performance"]);
        assert_eq!(read.active, "balanced");
        assert_eq!(profiles(&HashMap::new()), None);
    }
}
