// SPDX-License-Identifier: GPL-3.0-or-later
//! What the tray is told to draw, and when to warn -- worked out from what UPower says.
//!
//! No bus here: a [`Snapshot`] goes in, and a [`View`] and a warning [`Decision`] come out, so
//! every case is a unit test.
//!
//! # The icon
//!
//! UPower's own `IconName`, which it works out from the level and the charging state in the same
//! buckets as every desktop, so the icon agrees with anything else on the machine that shows one.
//! The tray finds it in the `wlrix` icon theme, which inherits Adwaita. Only when UPower gives
//! none is one worked out here, the same way.
//!
//! # Hidden, and when
//!
//! `Passive`, which the tray hides, when there is no battery -- every desktop -- and when the
//! person said `indicator = "hidden"`. Warnings do not depend on the icon: they still come.

use crate::config::Indicator;
use crate::i18n::{Language, tr};
use crate::upower::{Battery, Snapshot, State, Warning};

/// `Status`, as the StatusNotifierItem specification spells it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Passive,
    Active,
}

impl Status {
    pub fn as_str(self) -> &'static str {
        match self {
            Status::Passive => "Passive",
            Status::Active => "Active",
        }
    }
}

/// Everything the item publishes that can change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct View {
    pub status: Status,
    pub icon: String,
    pub title: String,
    pub body: String,
}

/// "2 h 10 min", "40 min", "less than a minute". `None` for no estimate.
pub fn duration(seconds: i64) -> Option<String> {
    if seconds <= 0 {
        return None;
    }
    let minutes = (seconds + 30) / 60;
    let language = Language::current();
    Some(match (minutes / 60, minutes % 60) {
        (0, 0) => tr("less than a minute", "1 分未満").to_owned(),
        (0, minutes) => language.pick(format!("{minutes} min"), format!("{minutes} 分")),
        (hours, 0) => language.pick(format!("{hours} h"), format!("{hours} 時間")),
        (hours, minutes) => language.pick(
            format!("{hours} h {minutes} min"),
            format!("{hours} 時間 {minutes} 分"),
        ),
    })
}

/// The battery's percentage, rounded, as it is shown everywhere.
pub fn percent(battery: &Battery) -> u32 {
    battery.percentage.round() as u32
}

/// One line saying what the battery is doing.
pub fn state_line(battery: &Battery) -> String {
    let language = Language::current();
    match battery.state {
        State::Discharging | State::PendingDischarge => match duration(battery.time_to_empty) {
            Some(left) => language.pick(format!("{left} remaining"), format!("残り {left}")),
            None => tr("On battery", "バッテリー駆動中").to_owned(),
        },
        State::Charging => match duration(battery.time_to_full) {
            Some(left) => language.pick(
                format!("Charging, full in {left}"),
                format!("充電中、満充電まで {left}"),
            ),
            None => tr("Charging", "充電中").to_owned(),
        },
        State::FullyCharged => tr("Fully charged", "充電完了").to_owned(),
        // A charge limit, or a battery too warm to charge: plugged in, holding where it is.
        State::PendingCharge => {
            tr("Plugged in, not charging", "電源に接続中、充電していません").to_owned()
        }
        State::Empty => tr("Empty", "空").to_owned(),
        State::Unknown => tr("Plugged in", "電源に接続中").to_owned(),
    }
}

/// The battery's level as the title of the tooltip and the menu: "Battery 73%".
pub fn level_line(battery: &Battery) -> String {
    Language::current().pick(
        format!("Battery {}%", percent(battery)),
        format!("バッテリー {}%", percent(battery)),
    )
}

/// What the tooltip and the menu say on a machine with no battery.
pub fn no_battery() -> &'static str {
    tr("No battery", "バッテリーなし")
}

/// The icon for a battery when UPower named none: tens of percent, charging or not.
fn fallback_icon(battery: &Battery) -> String {
    let level = ((battery.percentage / 10.0).round() as u32 * 10).min(100);
    let charging = matches!(battery.state, State::Charging | State::FullyCharged);
    match (level, charging) {
        (100, true) if battery.state == State::FullyCharged => {
            "battery-level-100-charged-symbolic".to_owned()
        }
        (level, true) => format!("battery-level-{level}-charging-symbolic"),
        (level, false) => format!("battery-level-{level}-symbolic"),
    }
}

/// Work out what to show.
pub fn view(snapshot: Option<&Snapshot>, setting: Indicator) -> View {
    let Some(battery) = snapshot.and_then(|snapshot| snapshot.battery.as_ref()) else {
        return View {
            status: Status::Passive,
            icon: "battery-missing-symbolic".to_owned(),
            title: no_battery().to_owned(),
            body: String::new(),
        };
    };
    View {
        status: match setting {
            Indicator::Auto => Status::Active,
            Indicator::Hidden => Status::Passive,
        },
        icon: if battery.icon.is_empty() {
            fallback_icon(battery)
        } else {
            battery.icon.clone()
        },
        title: level_line(battery),
        body: state_line(battery),
    }
}

/// A low-battery notification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Alert {
    pub summary: String,
    pub body: String,
    pub critical: bool,
}

/// What to do about warnings, given the level last warned about and the battery now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    Nothing,
    /// Warn, replacing any warning already up.
    Warn(Warning, Alert),
    /// Take the warning down: the machine has been plugged in.
    Clear,
}

/// Whether to warn.
///
/// Once per level, going down: low, then critical, then action, each replacing the last, so a
/// battery hovering at the edge of a level does not warn every time it crosses it. Plugging in
/// takes the warning down and re-arms all three.
pub fn decide(warned: Warning, battery: Option<&Battery>) -> Decision {
    let Some(battery) = battery else {
        return if warned > Warning::None {
            Decision::Clear
        } else {
            Decision::Nothing
        };
    };
    if !battery.state.on_battery() {
        return if warned > Warning::None {
            Decision::Clear
        } else {
            Decision::Nothing
        };
    }
    if battery.warning <= warned {
        return Decision::Nothing;
    }
    let language = Language::current();
    let left = duration(battery.time_to_empty)
        .map(|left| language.pick(format!(", about {left} left"), format!("、あと約 {left}")))
        .unwrap_or_default();
    let level = percent(battery);
    let alert = match battery.warning {
        Warning::None => return Decision::Nothing,
        Warning::Low => Alert {
            summary: tr("Battery low", "バッテリー残量が少なくなっています").to_owned(),
            body: language.pick(
                format!("{level}% remaining{left}. Plug in soon."),
                format!("残り {level}%{left}。早めに電源に接続してください。"),
            ),
            critical: false,
        },
        Warning::Critical => Alert {
            summary: tr("Battery critically low", "バッテリー残量がわずかです").to_owned(),
            body: language.pick(
                format!("{level}% remaining{left}. Plug in now, or save your work."),
                format!(
                    "残り {level}%{left}。すぐに電源に接続するか、作業内容を保存してください。"
                ),
            ),
            critical: true,
        },
        Warning::Action => Alert {
            summary: tr("Battery almost empty", "バッテリーがまもなく切れます").to_owned(),
            body: tr(
                "The computer is about to suspend or shut down to save what it can. Plug in now.",
                "保存できるものを守るため、まもなくサスペンドまたはシャットダウンします。すぐに電源に接続してください。",
            )
            .to_owned(),
            critical: true,
        },
    };
    Decision::Warn(battery.warning, alert)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn battery(state: State, percentage: f64, warning: Warning) -> Battery {
        Battery {
            state,
            percentage,
            time_to_empty: 2400,
            time_to_full: 1800,
            warning,
            icon: "battery-level-70-symbolic".into(),
        }
    }

    fn snapshot(battery: Battery) -> Snapshot {
        Snapshot {
            battery: Some(battery),
            profiles: None,
        }
    }

    #[test]
    fn no_battery_is_hidden() {
        assert_eq!(view(None, Indicator::Auto).status, Status::Passive);
        assert_eq!(
            view(Some(&Snapshot::default()), Indicator::Auto).status,
            Status::Passive
        );
    }

    #[test]
    fn a_battery_is_shown_unless_hidden() {
        let shot = snapshot(battery(State::Discharging, 73.4, Warning::None));
        let shown = view(Some(&shot), Indicator::Auto);
        assert_eq!(shown.status, Status::Active);
        assert_eq!(shown.icon, "battery-level-70-symbolic");
        assert_eq!(shown.title, "Battery 73%");
        assert_eq!(shown.body, "40 min remaining");
        assert_eq!(view(Some(&shot), Indicator::Hidden).status, Status::Passive);
    }

    #[test]
    fn each_state_says_what_it_is() {
        let line = |state| state_line(&battery(state, 50.0, Warning::None));
        assert_eq!(line(State::Charging), "Charging, full in 30 min");
        assert_eq!(line(State::FullyCharged), "Fully charged");
        assert_eq!(line(State::PendingCharge), "Plugged in, not charging");
        let mut fresh = battery(State::Discharging, 50.0, Warning::None);
        fresh.time_to_empty = 0;
        assert_eq!(state_line(&fresh), "On battery", "no estimate yet");
    }

    #[test]
    fn durations_read_as_people_say_them() {
        assert_eq!(duration(0), None);
        assert_eq!(duration(20), Some("less than a minute".into()));
        assert_eq!(duration(2400), Some("40 min".into()));
        assert_eq!(duration(7200), Some("2 h".into()));
        assert_eq!(duration(7800), Some("2 h 10 min".into()));
    }

    #[test]
    fn an_icon_is_worked_out_when_upower_gives_none() {
        let mut nameless = battery(State::Charging, 47.0, Warning::None);
        nameless.icon.clear();
        assert_eq!(
            fallback_icon(&nameless),
            "battery-level-50-charging-symbolic"
        );
        nameless.state = State::Discharging;
        assert_eq!(fallback_icon(&nameless), "battery-level-50-symbolic");
        nameless.state = State::FullyCharged;
        nameless.percentage = 100.0;
        assert_eq!(
            fallback_icon(&nameless),
            "battery-level-100-charged-symbolic"
        );
    }

    #[test]
    fn each_level_warns_once_going_down() {
        let low = battery(State::Discharging, 9.0, Warning::Low);
        let Decision::Warn(level, alert) = decide(Warning::None, Some(&low)) else {
            panic!("low should warn");
        };
        assert_eq!(level, Warning::Low);
        assert!(!alert.critical);
        assert!(alert.body.starts_with("9% remaining, about 40 min left"));
        // Still low: no second warning.
        assert_eq!(decide(Warning::Low, Some(&low)), Decision::Nothing);
        // Critical replaces it, and is critical.
        let critical = battery(State::Discharging, 4.0, Warning::Critical);
        assert!(matches!(
            decide(Warning::Low, Some(&critical)),
            Decision::Warn(Warning::Critical, Alert { critical: true, .. })
        ));
    }

    #[test]
    fn plugging_in_clears_and_rearms() {
        let charging = battery(State::Charging, 5.0, Warning::None);
        assert_eq!(decide(Warning::Critical, Some(&charging)), Decision::Clear);
        assert_eq!(decide(Warning::None, Some(&charging)), Decision::Nothing);
        // A battery reporting a warning level while charging is not warned about.
        let odd = battery(State::Charging, 5.0, Warning::Low);
        assert_eq!(decide(Warning::None, Some(&odd)), Decision::Nothing);
    }
}
