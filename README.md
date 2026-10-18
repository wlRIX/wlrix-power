# wlrix-power

Power status for the wlRIX desktop: the battery, as an icon in `wlrix-tray`. Its charge and whether it is charging,
a tooltip with the percentage and the time left, warnings when it runs low, a menu to switch power modes -- and nothing
at all on a machine with no battery.

- **Language:** Rust
- **License:** GPL-3.0-or-later

IRIX ran on workstations that were plugged into a wall. This is one of the pieces that makes wlRIX usable on a laptop,
beside `wlrix-network` for Wi-Fi and `wlrix-osd` for notifications and the volume and brightness bar. It is
`wlrix-power` rather than wlrix-battery because the menu also holds the power modes.

## What it is

A daemon with no window. It reads **UPower** on the system bus and publishes a **StatusNotifierItem** on the session
bus, which the tray draws -- the same way `wlrix-network` gets there. It needs no compositor and no change to the tray.

What it shows is UPower's *display device*, `/org/freedesktop/UPower/devices/DisplayDevice`: the machine's batteries
added together, which is one battery on almost every laptop and the right total on the few with two.

| State                       | Tooltip                                      |
|-----------------------------|----------------------------------------------|
| on battery                  | "Battery 73%" / "2 h 55 min remaining"       |
| charging                    | "Battery 40%" / "Charging, full in 1 h 5 min" |
| full                        | "Battery 100%" / "Fully charged"             |
| plugged in, holding (a charge limit) | "Battery 80%" / "Plugged in, not charging" |

The icon is UPower's own `IconName` (`battery-level-70-symbolic`, `battery-level-10-charging-symbolic`, ...), worked
out in the same buckets as every desktop, so it agrees with anything else on the machine that shows one. The tray finds
it in the `wlrix` icon theme, which inherits Adwaita.

**The item is `Passive`, which the tray hides, when there is no battery** -- every desktop -- and when UPower is not
running. A UPS or a wireless mouse's battery is not the display device and does not show.

## Warnings

UPower decides when the battery is low, critical, or at the *action* level where it suspends or powers off the machine
itself -- from `PercentageLow`, `PercentageCritical` and `PercentageAction` in `/etc/UPower/UPower.conf`, or from time
left. This daemon tells the person, through the session's notification daemon (`wlrix-osd`):

- **Once per level, going down.** Low, then critical, then action, each replacing the one before, so a battery hovering
  at the edge of a level does not warn every time it crosses it.
- **Low is a normal notification; critical and action are critical** (`urgency` 2), which `wlrix-osd` keeps on screen
  until dismissed.
- **Plugging in takes the warning down** and re-arms every level. So does the daemon stopping.

Warnings do not depend on the icon: `indicator = "hidden"` still warns.

## The menu

A click on the icon opens it, drawn by the tray as a 4Dwm menu:

```text
Battery 73%
2 h 55 min remaining
--------------------
( ) Power Saver
(*) Balanced
( ) Performance
```

The power modes are **power-profiles-daemon**'s (`org.freedesktop.UPower.PowerProfiles`), listed only when it runs,
and only the ones it offers -- Performance needs hardware support. Its polkit policy lets the active session switch
without a password. A refusal is a notification; a change made elsewhere (`powerprofilesctl`, another desktop) moves
the radio button.

The menu is `com.canonical.dbusmenu` at `/MenuBar`, served in full, with ids fixed per row and per mode.

## Configuration

`~/.config/wlrix/power.toml`, then `/etc/wlrix/power.toml`. The first file found wins outright, unknown keys are
refused, and `wlrix-power --check-config <path>` says whether a file would be accepted.

```toml
indicator = "auto"   # auto | hidden
warnings = true      # low-battery notifications
```

`SIGHUP` re-reads the file and `SIGTERM` stops the daemon; the pidfile is `$XDG_RUNTIME_DIR/wlrix-power.pid`. Both keys
are in the settings daemon's schema, so Desktop Settings can change them.

## Using it

`wlrix-session` starts it. `wlrix-power --status` reads UPower once and prints what the icon would show:

```console
$ wlrix-power --status
status:  Active
icon:    battery-level-70-symbolic
title:   Battery 73%
         2 h 55 min remaining
state:   Discharging, warning level None
mode:    balanced (of power-saver, balanced, performance)
```

The item's `Id` is `wlrix-power`, for ordering or hiding it with `[[item]]` in `tray.toml`.

## How it stays current

**Every signal means "read it all again"**, as in `wlrix-network`: a `PropertiesChanged` under `/org/freedesktop/UPower`
or either daemon's name changing owner arms a quarter-second timer, and when it fires both daemons are read again. A
battery reports its level, rate and time left as separate changes; that is one read.

**The tray restarting** loses every registration. The daemon watches `org.kde.StatusNotifierWatcher` change owner and
registers again. Every call it makes times out after ten seconds.

## Testing it

What is shown, the menu and when to warn are unit-tested case by case, as is reading UPower's property maps.

```console
$ cargo test && cargo clippy --all-targets -- -D warnings && cargo fmt --all --check
```

A machine with no battery -- or one whose battery should not have to be run down -- can test everything with
`examples/fake_upower.rs`, a stand-in UPower and power-profiles-daemon. zbus connects to whatever
`DBUS_SYSTEM_BUS_ADDRESS` names, so start a private bus and point both at it; never the real system bus:

```console
$ export DBUS_SYSTEM_BUS_ADDRESS=$(dbus-daemon --session --print-address --fork)
$ cargo run --example fake_upower       # then type: battery 9 discharging low
$ wlrix-power                           # in another terminal, with the same address
```

Its input lines are `battery <percent> <charging|discharging|full|pending> [low|critical|action]`, `none`,
`profile <name>` and `no-profiles`. Run the daemon on a private session bus as well (`env -u WAYLAND_DISPLAY
dbus-run-session`), with a nested compositor, `wlrix-tray` and `wlrix-osd` in it, so the live session's tray and
notifications are not involved. The menu can be driven with `busctl --user call <item name> /MenuBar
com.canonical.dbusmenu Event isvu <id> clicked i 0 0`.
