// SPDX-License-Identifier: GPL-3.0-or-later
//! The menu the battery icon opens, as a tree of rows -- built from a snapshot, no bus.
//!
//! ```text
//! Battery 73%                (headings; cannot be chosen)
//! 40 min remaining
//! ---------------------------
//! ( ) Power Saver
//! (*) Balanced
//! ( ) Performance
//! ```
//!
//! The power modes are power-profiles-daemon's, listed only when it is running. The tray draws
//! the menu as a 4Dwm menu from `com.canonical.dbusmenu`, which [`crate::dbusmenu`] serves.
//!
//! Ids are fixed per row and per profile name, so a click that arrives after the menu changed
//! still means what was clicked -- the same rule as `wlrix-network`'s menu.

use crate::i18n::tr;
use crate::indicator::{level_line, no_battery, state_line};
use crate::upower::Snapshot;

/// What choosing a row does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Switch to this power mode.
    Profile(String),
}

/// A row's check box or radio button.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Toggle {
    Check(bool),
    Radio(bool),
}

/// One row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub id: i32,
    pub label: String,
    pub enabled: bool,
    pub separator: bool,
    pub toggle: Option<Toggle>,
    pub children: Vec<Row>,
    pub action: Option<Action>,
}

impl Row {
    fn new(id: i32, label: impl Into<String>) -> Self {
        Self {
            id,
            label: label.into(),
            enabled: true,
            separator: false,
            toggle: None,
            children: Vec::new(),
            action: None,
        }
    }

    fn heading(id: i32, label: impl Into<String>) -> Self {
        Self {
            enabled: false,
            ..Self::new(id, label)
        }
    }

    fn separator(id: i32) -> Self {
        Self {
            separator: true,
            ..Self::new(id, "")
        }
    }

    /// The label as dbusmenu carries it, with `_` doubled so it is not read as a mnemonic.
    pub fn wire_label(&self) -> String {
        self.label.replace('_', "__")
    }

    /// This row or one under it, by id.
    pub fn find(&self, id: i32) -> Option<&Row> {
        if self.id == id {
            return Some(self);
        }
        self.children.iter().find_map(|child| child.find(id))
    }
}

const LEVEL: i32 = 1;
const STATE: i32 = 2;
const PROFILES_SEPARATOR: i32 = 3;
/// The three profiles power-profiles-daemon has, at fixed ids; any other name it ever adds gets
/// one after these.
const FIRST_PROFILE: i32 = 10;
const KNOWN_PROFILES: [&str; 3] = ["power-saver", "balanced", "performance"];

/// A profile as a person reads it.
fn profile_label(name: &str) -> String {
    match name {
        "power-saver" => tr("Power Saver", "省電力").to_owned(),
        "balanced" => tr("Balanced", "バランス").to_owned(),
        "performance" => tr("Performance", "パフォーマンス").to_owned(),
        other => other.to_owned(),
    }
}

/// Build the menu's root, whose children are the rows.
pub fn build(snapshot: Option<&Snapshot>) -> Row {
    let mut root = Row::new(0, "");
    match snapshot.and_then(|snapshot| snapshot.battery.as_ref()) {
        Some(battery) => {
            root.children.push(Row::heading(LEVEL, level_line(battery)));
            root.children.push(Row::heading(STATE, state_line(battery)));
        }
        None => root.children.push(Row::heading(LEVEL, no_battery())),
    }
    if let Some(profiles) = snapshot.and_then(|snapshot| snapshot.profiles.as_ref()) {
        root.children.push(Row::separator(PROFILES_SEPARATOR));
        for (index, name) in profiles.available.iter().enumerate() {
            let id = KNOWN_PROFILES
                .iter()
                .position(|known| known == name)
                .map_or(
                    FIRST_PROFILE + KNOWN_PROFILES.len() as i32 + index as i32,
                    |at| FIRST_PROFILE + at as i32,
                );
            root.children.push(Row {
                toggle: Some(Toggle::Radio(*name == profiles.active)),
                action: Some(Action::Profile(name.clone())),
                ..Row::new(id, profile_label(name))
            });
        }
    }
    root
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::upower::{Battery, Profiles, State, Warning};

    fn snapshot(profiles: bool) -> Snapshot {
        Snapshot {
            battery: Some(Battery {
                state: State::Discharging,
                percentage: 73.0,
                time_to_empty: 2400,
                time_to_full: 0,
                warning: Warning::None,
                icon: String::new(),
            }),
            profiles: profiles.then(|| Profiles {
                available: vec![
                    "power-saver".into(),
                    "balanced".into(),
                    "performance".into(),
                ],
                active: "balanced".into(),
            }),
        }
    }

    fn labels(row: &Row) -> Vec<String> {
        row.children
            .iter()
            .map(|child| child.label.clone())
            .collect()
    }

    #[test]
    fn the_level_and_the_power_modes() {
        let menu = build(Some(&snapshot(true)));
        assert_eq!(
            labels(&menu),
            [
                "Battery 73%",
                "40 min remaining",
                "",
                "Power Saver",
                "Balanced",
                "Performance"
            ]
        );
        assert!(!menu.children[0].enabled && !menu.children[1].enabled);
        assert_eq!(menu.children[4].toggle, Some(Toggle::Radio(true)));
        assert_eq!(menu.children[3].toggle, Some(Toggle::Radio(false)));
        assert_eq!(
            menu.children[5].action,
            Some(Action::Profile("performance".into()))
        );
    }

    #[test]
    fn no_power_profiles_daemon_no_modes() {
        assert_eq!(
            labels(&build(Some(&snapshot(false)))),
            ["Battery 73%", "40 min remaining"]
        );
    }

    #[test]
    fn profile_ids_do_not_move_when_the_list_does() {
        let mut shot = snapshot(true);
        let before = build(Some(&shot)).children[5].id;
        shot.profiles.as_mut().unwrap().available.remove(0);
        let after = build(Some(&shot))
            .children
            .iter()
            .find(|row| row.label == "Performance")
            .unwrap()
            .id;
        assert_eq!(before, after);
    }
}
