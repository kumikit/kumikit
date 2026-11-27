//! Desktop window chrome configuration and tiling window manager detection
//! (`docs/spec/06-client.md#shell-rust-kumikit-shell`).

use kumikit_protocol::local::{Env, Platform};
use serde::Serialize;

/// Environment variables whose non-empty presence identifies a tiling window manager.
const TILING_SOCKET_VARS: &[&str] = &[
    "HYPRLAND_INSTANCE_SIGNATURE",
    "SWAYSOCK",
    "NIRI_SOCKET",
    "I3SOCK",
    "BSPWM_SOCKET",
    "RIVER_SOCKET",
];

/// Desktop session tokens (`XDG_CURRENT_DESKTOP`, `XDG_SESSION_DESKTOP`, `DESKTOP_SESSION`)
/// that identify a tiling window manager (matched case-insensitively on token boundaries).
const TILING_DESKTOP_TOKENS: &[&str] = &[
    "hyprland",
    "sway",
    "i3",
    "niri",
    "river",
    "bspwm",
    "awesome",
    "dwm",
    "xmonad",
    "qtile",
    "herbstluftwm",
    "leftwm",
    "spectrwm",
    "stumpwm",
    "ratpoison",
    "miracle-wm",
];

/// Window decoration and control metadata exposed to the UI kernel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowChrome {
    /// Whether the shell removed OS window decorations (`decorations(false)`).
    pub borderless: bool,
    /// Whether macOS native overlay traffic lights sit inside the top-left of the bar.
    pub native_overlay: bool,
    /// Whether a Linux tiling window manager was detected.
    pub tiling: bool,
    /// Whether the UI should render thin edge-resize strips (undecorated Linux stacking DE,
    /// where GTK3 removes native edge resize handles along with decorations).
    pub custom_resize: bool,
}

impl WindowChrome {
    /// Computes window chrome settings for the current platform and environment.
    pub fn current(env: Env<'_>) -> Self {
        Self::for_platform(Platform::current(), env)
    }

    /// Computes window chrome settings for `platform` and `env`.
    pub fn for_platform(platform: Platform, env: Env<'_>) -> Self {
        match platform {
            Platform::MacOs => WindowChrome {
                borderless: false,
                native_overlay: true,
                tiling: false,
                custom_resize: false,
            },
            Platform::Windows => WindowChrome {
                borderless: true,
                native_overlay: false,
                tiling: false,
                custom_resize: false,
            },
            Platform::Linux => {
                let tiling = detect_tiling_wm(env);
                WindowChrome {
                    borderless: true,
                    native_overlay: false,
                    tiling,
                    custom_resize: !tiling,
                }
            }
        }
    }
}

/// Returns `true` when Linux session environment variables indicate a tiling window manager.
pub fn detect_tiling_wm(env: Env<'_>) -> bool {
    if TILING_SOCKET_VARS
        .iter()
        .any(|var| env(var).is_some_and(|v| !v.trim().is_empty()))
    {
        return true;
    }
    for var in [
        "XDG_CURRENT_DESKTOP",
        "XDG_SESSION_DESKTOP",
        "DESKTOP_SESSION",
    ] {
        let Some(value) = env(var) else {
            continue;
        };
        for token in value
            .split(|c: char| c == ':' || c == ';' || c == ',' || c.is_whitespace())
            .filter(|t| !t.is_empty())
        {
            let lower = token.to_ascii_lowercase();
            if TILING_DESKTOP_TOKENS.contains(&lower.as_str()) {
                return true;
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    fn env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> + use<> {
        let map: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        move |k| map.get(k).cloned()
    }

    #[test]
    fn detects_tiling_compositors_from_socket_vars() {
        for var in TILING_SOCKET_VARS {
            let e = env(&[(var, "/run/user/1000/wm.sock")]);
            assert!(detect_tiling_wm(&e), "expected {var} to detect tiling WM");
        }
        let empty = env(&[("HYPRLAND_INSTANCE_SIGNATURE", "   ")]);
        assert!(!detect_tiling_wm(&empty));
    }

    #[test]
    fn detects_tiling_wms_from_desktop_session_tokens() {
        let e = env(&[("XDG_CURRENT_DESKTOP", "Hyprland")]);
        assert!(detect_tiling_wm(&e));

        let e = env(&[("XDG_CURRENT_DESKTOP", "ubuntu:sway")]);
        assert!(detect_tiling_wm(&e));

        let e = env(&[("XDG_SESSION_DESKTOP", "i3")]);
        assert!(detect_tiling_wm(&e));

        let e = env(&[("DESKTOP_SESSION", "niri")]);
        assert!(detect_tiling_wm(&e));
    }

    #[test]
    fn stacking_desktops_are_not_marked_tiling() {
        for value in [
            "GNOME",
            "ubuntu:GNOME",
            "KDE",
            "XFCE",
            "X-Cinnamon",
            "COSMIC",
        ] {
            let e = env(&[("XDG_CURRENT_DESKTOP", value)]);
            assert!(
                !detect_tiling_wm(&e),
                "stacking desktop {value} should not be detected as tiling"
            );
        }
        let empty = env(&[]);
        assert!(!detect_tiling_wm(&empty));
    }

    #[test]
    fn platform_chrome_defaults() {
        let hypr = env(&[("HYPRLAND_INSTANCE_SIGNATURE", "abc")]);
        assert_eq!(
            WindowChrome::for_platform(Platform::Linux, &hypr),
            WindowChrome {
                borderless: true,
                native_overlay: false,
                tiling: true,
                custom_resize: false,
            }
        );

        let gnome = env(&[("XDG_CURRENT_DESKTOP", "GNOME")]);
        assert_eq!(
            WindowChrome::for_platform(Platform::Linux, &gnome),
            WindowChrome {
                borderless: true,
                native_overlay: false,
                tiling: false,
                custom_resize: true,
            }
        );

        assert_eq!(
            WindowChrome::for_platform(Platform::MacOs, &hypr),
            WindowChrome {
                borderless: false,
                native_overlay: true,
                tiling: false,
                custom_resize: false,
            }
        );

        assert_eq!(
            WindowChrome::for_platform(Platform::Windows, &hypr),
            WindowChrome {
                borderless: true,
                native_overlay: false,
                tiling: false,
                custom_resize: false,
            }
        );
    }
}
