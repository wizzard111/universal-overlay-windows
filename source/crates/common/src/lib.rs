//! Shared code for the overlay and the settings app: config model,
//! hotkey parsing, font discovery and IPC constants.

pub mod config;
pub mod fonts;
pub mod hotkey;
pub mod sensor;
pub mod sensord;

/// Window class of the overlay's main window. The settings app finds the
/// running overlay by this class and posts [`MSG_RELOAD`] to it.
pub const OVERLAY_WINDOW_CLASS: &str = "UniversalOverlay.Main";

/// WM_APP + 1: "config file changed, reload it".
pub const MSG_RELOAD: u32 = 0x8000 + 1;

/// WM_APP + 3: "toggle visibility" (sent by `universal-overlay --toggle`).
pub const MSG_TOGGLE: u32 = 0x8000 + 3;

/// Executable names (without extension) of the two programs.
pub const OVERLAY_EXE: &str = "universal-overlay";
pub const SETTINGS_EXE: &str = "universal-overlay-settings";
pub const UPDATER_EXE: &str = "universal-overlay-updater";

/// WM_APP + 4: "quit" (Windows; the updater uses it before replacing files).
pub const MSG_QUIT: u32 = 0x8000 + 4;

/// Program version (all binaries share the workspace version).
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// GitHub repository ("owner/name") the updater pulls releases from.
/// Set at build time: `UO_GITHUB_REPO=owner/name cargo build --release`.
pub const GITHUB_REPO: &str = match option_env!("UO_GITHUB_REPO") {
    Some(r) => r,
    None => "wizzard111/universal-overlay-windows",
};

/// Release archive name for this platform, e.g.
/// `universal-overlay-windows-x86_64.zip`, `universal-overlay-linux-x86_64.tar.gz`,
/// `universal-overlay-macos-universal.tar.gz`.
pub fn release_asset_name() -> String {
    format!("universal-overlay-windows-{}.zip", std::env::consts::ARCH)
}

/// Compare dotted versions ("1.2.10" > "1.2.9"); a leading "v" is ignored.
pub fn version_newer(candidate: &str, current: &str) -> bool {
    let parse = |s: &str| -> Vec<u64> {
        s.trim()
            .trim_start_matches(['v', 'V'])
            .split(['.', '-', '+'])
            .map_while(|p| p.parse().ok())
            .collect()
    };
    let (a, b) = (parse(candidate), parse(current));
    for i in 0..a.len().max(b.len()) {
        let (x, y) = (
            a.get(i).copied().unwrap_or(0),
            b.get(i).copied().unwrap_or(0),
        );
        if x != y {
            return x > y;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    #[test]
    fn versions() {
        assert!(super::version_newer("v0.2.0", "0.1.9"));
        assert!(super::version_newer("1.2.10", "1.2.9"));
        assert!(!super::version_newer("1.2.0", "1.2.0"));
        assert!(!super::version_newer("v1.0", "1.0.1"));
    }
}
