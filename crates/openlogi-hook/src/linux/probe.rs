//! Flow's pointer and display queries, selected for this desktop session.
//!
//! X11 uses root-window pixels. Hyprland exposes a global cursor and monitor
//! layout over its IPC socket, which is the only Wayland compositor interface
//! this probe can use today; other Wayland sessions have no global cursor
//! position, so they report no pointer.
//!
//! Both backends share one coordinate space: the compositor's global layout
//! coordinates, in which the displays tile without overlap.

mod hyprland;
mod x11;

use crate::linux::foreground::{SessionKind, detect_session_kind};
use crate::{DesktopPoint, DisplayBounds, PointerState, WarpError};

/// The pointer probe for the current session.
pub(crate) struct Probe {
    backend: Backend,
}

enum Backend {
    X11(Box<x11::Probe>),
    Hyprland(Box<hyprland::Probe>),
}

impl Probe {
    pub(crate) fn connect() -> Option<Self> {
        match detect_session_kind() {
            SessionKind::Wayland => hyprland::Probe::connect().map(|probe| Self {
                backend: Backend::Hyprland(Box::new(probe)),
            }),
            SessionKind::X11 => x11::Probe::connect().map(|probe| Self {
                backend: Backend::X11(Box::new(probe)),
            }),
            // Without a session type, trust whichever compositor answers.
            SessionKind::Unknown => hyprland::Probe::connect()
                .map(|probe| Self {
                    backend: Backend::Hyprland(Box::new(probe)),
                })
                .or_else(|| {
                    x11::Probe::connect().map(|probe| Self {
                        backend: Backend::X11(Box::new(probe)),
                    })
                }),
        }
    }

    pub(crate) fn state(&mut self) -> Option<PointerState> {
        match &mut self.backend {
            Backend::X11(probe) => probe.state(),
            Backend::Hyprland(probe) => probe.state(),
        }
    }

    pub(crate) fn displays(&mut self) -> Vec<DisplayBounds> {
        match &mut self.backend {
            Backend::X11(probe) => probe.displays(),
            Backend::Hyprland(probe) => probe.displays(),
        }
    }

    pub(crate) fn warp(&mut self, to: DesktopPoint) -> Result<(), WarpError> {
        match &mut self.backend {
            Backend::X11(probe) => probe.warp(to),
            Backend::Hyprland(probe) => probe.warp(to),
        }
    }

    pub(crate) fn reports_modifiers(&self) -> bool {
        match &self.backend {
            Backend::X11(_) => true,
            Backend::Hyprland(probe) => probe.reports_modifiers(),
        }
    }

    pub(crate) fn set_modifiers_required(&mut self, required: bool) {
        match &mut self.backend {
            // QueryPointer returns the mask with the position, so there is
            // nothing extra to skip.
            Backend::X11(_) => {}
            Backend::Hyprland(probe) => probe.set_modifiers_required(required),
        }
    }
}
