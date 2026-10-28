// SPDX-License-Identifier: GPL-3.0-or-later
//! wlRIX's power status.
//!
//! The battery, published as a StatusNotifierItem so `wlrix-tray` draws it: an icon with the
//! charge, a tooltip with the percentage and the time left, and a menu with the power modes of
//! power-profiles-daemon. Low-battery warnings go to the session's notification daemon. On a
//! machine with no battery the item is `Passive`, which the tray hides.
//!
//! - [`upower`] reads UPower and power-profiles-daemon on the system bus and says when they change.
//! - [`indicator`] turns what it read into an icon, a status, a tooltip and when to warn. No bus;
//!   unit-tested.
//! - [`menu`] builds the menu from what was read. No bus; unit-tested.
//! - [`sni`] is the item on the session bus, and its registration with the tray; [`dbusmenu`]
//!   serves the menu beside it.
//! - [`daemon`] is the loop between them.

pub mod config;
pub mod daemon;
pub mod dbusmenu;
pub mod i18n;
pub mod indicator;
pub mod menu;
pub mod pidfile;
pub mod signals;
pub mod sni;
pub mod upower;
pub mod xdg;
