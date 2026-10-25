//! Centralized hidden child-process construction for the panel.
//!
//! Every internal helper (`rundll32.exe` browser launches, …) must stay
//! invisible on Windows GUI sessions. `MCPANEL_CONSOLE=1` only affects the
//! panel's own console (via `windows_subsystem` in `main.rs` + `build.rs`);
//! helpers created here always stay hidden.

use std::ffi::OsStr;

/// Windows `CREATE_NO_WINDOW` (`0x0800_0000`).
///
/// Only referenced by the Windows flag path; the portable unit test below
/// still pins the SDK value on every platform, hence the targeted allow
/// instead of a `cfg(windows)` gate.
#[cfg_attr(not(windows), allow(dead_code))]
pub const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Apply `CREATE_NO_WINDOW` to a std child command. No-op on other OSes.
pub fn hide(command: &mut std::process::Command) -> &mut std::process::Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    #[cfg(not(windows))]
    {
        let _ = command;
    }
    command
}

/// Build a std command that stays invisible on Windows.
pub fn hidden_command(program: impl AsRef<OsStr>) -> std::process::Command {
    let mut command = std::process::Command::new(program);
    hide(&mut command);
    command
}

/// Open `url` in the user's browser without a shell and without flashing a
/// console window on Windows.
///
/// Uses a direct executable invocation (`rundll32.exe` on Windows,
/// `xdg-open` family on Linux) so a URL can never become shell syntax.
pub fn open_browser(url: &str) {
    #[cfg(windows)]
    {
        // rundll32 is a console program; without CREATE_NO_WINDOW it allocates
        // a visible console window even when the panel itself has none.
        let _ = hidden_command("rundll32.exe")
            .args(["url.dll,FileProtocolHandler", url])
            .spawn();
        return;
    }
    #[cfg(all(target_os = "linux", not(target_env = "musl")))]
    {
        let candidates: [&[&str]; 4] = [
            &["xdg-open", url],
            &["gio", "open", url],
            &["kde-open5", url],
            &["sensible-browser", url],
        ];
        for candidate in candidates {
            if hidden_command(candidate[0])
                .args(&candidate[1..])
                .spawn()
                .is_ok()
            {
                return;
            }
        }
        tracing::warn!("could not open browser for setup/recovery");
        return;
    }
    #[cfg(not(any(windows, all(target_os = "linux", not(target_env = "musl")))))]
    {
        let _ = url;
        tracing::warn!("browser opening not supported on this platform");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flag_matches_the_windows_sdk_value() {
        assert_eq!(CREATE_NO_WINDOW, 0x0800_0000);
    }

    #[test]
    fn hidden_command_targets_the_requested_program() {
        let command = hidden_command("rundll32.exe");
        assert_eq!(command.get_program(), OsStr::new("rundll32.exe"));
    }

    #[test]
    fn hide_is_idempotent() {
        let mut command = std::process::Command::new("rundll32.exe");
        hide(&mut command);
        hide(&mut command);
        assert_eq!(command.get_program(), OsStr::new("rundll32.exe"));
    }
}
