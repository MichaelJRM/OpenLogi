//! Flow's pointer and display queries on a Hyprland session.
//!
//! Hyprland is the one Wayland compositor that exposes a global cursor
//! position, the monitor layout, a cursor warp, and the held modifier keys
//! over IPC. The socket is the same one `hyprctl` uses; every command opens
//! its own connection and Hyprland closes it after answering. `cursorpos` and
//! the monitor positions share the global layout coordinate space used by
//! [`DisplayBounds`].
//!
//! The modifier query needs Hyprland's Lua API (`repl` + `hl.is_key_down`),
//! which arrived with the Lua configuration manager. On an older Hyprland the
//! probe still reads the cursor, but reports that modifiers are unavailable,
//! so Flow's Control guard keeps the edges disarmed instead of silently
//! ignoring the guard.

use std::io::{self, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Deserialize;
use tracing::debug;

use crate::{DesktopPoint, DisplayBounds, KeyModifiers, PointerState, WarpError};

/// Longest wait for one Hyprland IPC answer. The pointer thread calls this on
/// every motion sample, so a wedged compositor must not wedge the thread.
const IPC_TIMEOUT: Duration = Duration::from_millis(500);

/// One round trip that answers all four modifiers as a `1010` bit string:
/// shift, control, option/Alt, command/Super. `repl` prints the value; `eval`
/// would only print `ok`.
const MODIFIER_QUERY: &str = r#"repl local b=function(x) return x and 1 or 0 end return string.format("%d%d%d%d", b(hl.is_key_down("Shift_L") or hl.is_key_down("Shift_R")), b(hl.is_key_down("Control_L") or hl.is_key_down("Control_R")), b(hl.is_key_down("Alt_L") or hl.is_key_down("Alt_R")), b(hl.is_key_down("Super_L") or hl.is_key_down("Super_R")))"#;

/// The Hyprland probe: one IPC socket and whether modifiers can be read.
pub(crate) struct Probe {
    socket: PathBuf,
    reports_modifiers: bool,
    /// Set by the pointer loop when its Control guard is enabled.
    modifiers_required: bool,
}

impl Probe {
    pub(crate) fn connect() -> Option<Self> {
        let socket = socket_path()?;
        // A live query proves the socket belongs to a running Hyprland, not a
        // stale instance directory.
        parse_position(&query(&socket, "cursorpos").ok()?)?;
        let reports_modifiers = query_modifiers(&socket).is_some();
        Some(Self {
            socket,
            reports_modifiers,
            modifiers_required: false,
        })
    }

    pub(crate) fn state(&mut self) -> Option<PointerState> {
        let position = parse_position(&query(&self.socket, "cursorpos").ok()?)?;
        let modifiers = if self.modifiers_required {
            query_modifiers(&self.socket).unwrap_or_default()
        } else {
            KeyModifiers::default()
        };
        Some(PointerState {
            position,
            modifiers,
        })
    }

    pub(crate) fn displays(&mut self) -> Vec<DisplayBounds> {
        query(&self.socket, "j/monitors")
            .ok()
            .and_then(|response| parse_monitors(&response))
            .unwrap_or_default()
    }

    pub(crate) fn warp(&mut self, to: DesktopPoint) -> Result<(), WarpError> {
        let (x, y) = (pixel(to.x), pixel(to.y));
        // Hyprland 0.56 parses a Lua config's dispatches as Lua, so the legacy
        // string dispatcher fails there. It is still the only one older
        // Hyprlands know, so try it first and fall back to the Lua dispatcher.
        match query(&self.socket, &legacy_warp(x, y)) {
            Ok(response) if response.trim() == "ok" => return Ok(()),
            Ok(response) => {
                debug!(
                    response = response.trim(),
                    "the legacy Hyprland warp failed"
                );
            }
            Err(error) => return Err(WarpError::Rejected(error)),
        }
        match query(&self.socket, &lua_warp(x, y)) {
            Ok(response) if response.trim() == "ok" => Ok(()),
            Ok(response) => Err(WarpError::Rejected(io::Error::other(
                response.trim().to_owned(),
            ))),
            Err(error) => Err(WarpError::Rejected(error)),
        }
    }

    /// Whether [`Self::state`] reports the held modifier keys.
    pub(crate) const fn reports_modifiers(&self) -> bool {
        self.reports_modifiers
    }

    /// Skip the modifier query unless the caller gates on a modifier.
    pub(crate) fn set_modifiers_required(&mut self, required: bool) {
        self.modifiers_required = required;
    }
}

fn legacy_warp(x: i32, y: i32) -> String {
    format!("dispatch movecursor {x} {y}")
}

fn lua_warp(x: i32, y: i32) -> String {
    format!("eval hl.dispatch(hl.dsp.cursor.move({{x={x}, y={y}}}))")
}

/// The running instance's command socket.
///
/// `HYPRLAND_INSTANCE_SIGNATURE` is exported into the systemd user
/// environment, where the agent runs. The `/tmp` path is where Hyprland put
/// its socket before it moved under `XDG_RUNTIME_DIR`.
fn socket_path() -> Option<PathBuf> {
    let signature = std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE")?;
    let mut candidates = Vec::with_capacity(2);
    if let Some(runtime) = std::env::var_os("XDG_RUNTIME_DIR") {
        candidates.push(
            Path::new(&runtime)
                .join("hypr")
                .join(&signature)
                .join(".socket.sock"),
        );
    }
    candidates.push(Path::new("/tmp/hypr").join(&signature).join(".socket.sock"));
    candidates.into_iter().find(|path| path.exists())
}

/// Run one Hyprland IPC command and return its whole answer.
fn query(socket: &Path, command: &str) -> io::Result<String> {
    let mut stream = UnixStream::connect(socket)?;
    stream.set_read_timeout(Some(IPC_TIMEOUT))?;
    stream.set_write_timeout(Some(IPC_TIMEOUT))?;
    stream.write_all(command.as_bytes())?;
    // Hyprland closes the connection after one command, so reading to EOF
    // bounds the answer without a length prefix.
    let mut response = String::new();
    stream.read_to_string(&mut response)?;
    Ok(response)
}

/// Parse `cursorpos`, which Hyprland formats as `"x, y"`.
fn parse_position(response: &str) -> Option<DesktopPoint> {
    let (x, y) = response.trim().split_once(',')?;
    let point = DesktopPoint {
        x: x.trim().parse().ok()?,
        y: y.trim().parse().ok()?,
    };
    (point.x.is_finite() && point.y.is_finite()).then_some(point)
}

fn query_modifiers(socket: &Path) -> Option<KeyModifiers> {
    parse_modifiers(&query(socket, MODIFIER_QUERY).ok()?)
}

fn parse_modifiers(response: &str) -> Option<KeyModifiers> {
    let response = response.trim().as_bytes();
    if response.len() != 4 {
        return None;
    }
    let bit = |index: usize| match response.get(index) {
        Some(b'0') => Some(false),
        Some(b'1') => Some(true),
        _ => None,
    };
    Some(KeyModifiers {
        shift: bit(0)?,
        control: bit(1)?,
        option: bit(2)?,
        command: bit(3)?,
    })
}

#[derive(Deserialize)]
struct Monitor {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    #[serde(default = "one")]
    scale: f64,
    #[serde(default)]
    transform: u8,
    #[serde(default)]
    disabled: bool,
}

const fn one() -> f64 {
    1.0
}

/// Parse the active monitors from `j/monitors`.
///
/// Hyprland reports the physical pixel size and the scale. Its layout
/// position and the cursor are in logical units, so a monitor occupies
/// `width / scale` there; an odd transform swaps the two sides.
fn parse_monitors(response: &str) -> Option<Vec<DisplayBounds>> {
    let monitors: Vec<Monitor> = serde_json::from_str(response).ok()?;
    Some(
        monitors
            .into_iter()
            .filter(|monitor| !monitor.disabled)
            .filter_map(|monitor| bounds(&monitor))
            .collect(),
    )
}

fn bounds(monitor: &Monitor) -> Option<DisplayBounds> {
    if !(monitor.x.is_finite()
        && monitor.y.is_finite()
        && monitor.width.is_finite()
        && monitor.height.is_finite())
    {
        return None;
    }
    let scale = if monitor.scale.is_finite() && monitor.scale > 0.0 {
        monitor.scale
    } else {
        1.0
    };
    let (width, height) = if monitor.transform % 2 == 1 {
        (monitor.height, monitor.width)
    } else {
        (monitor.width, monitor.height)
    };
    let width = (width / scale).round();
    let height = (height / scale).round();
    (width > 0.0 && height > 0.0).then_some(DisplayBounds {
        x: monitor.x,
        y: monitor.y,
        width,
        height,
    })
}

/// The nearest layout pixel, clamped to what the IPC command can carry.
#[expect(
    clippy::cast_possible_truncation,
    reason = "the rounded value is clamped to the i32 range before conversion"
)]
fn pixel(coordinate: f64) -> i32 {
    let rounded = if coordinate.is_finite() {
        coordinate.round()
    } else {
        0.0
    };
    rounded.clamp(f64::from(i32::MIN), f64::from(i32::MAX)) as i32
}

#[cfg(test)]
mod tests;
