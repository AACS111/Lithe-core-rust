//! Process creation and exit status for background tools.

use std::ffi::OsStr;
use std::process::{Command, ExitStatus};

/// The Windows `CREATE_NO_WINDOW` process creation flag.
pub const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Starts a console program without a console window.
///
/// Core runs tools such as Git as background tasks; on Windows each one would
/// otherwise attach a console and can briefly open the user's terminal.
/// Replaces creation flags set on `command` before. Unix children have no
/// window, so nothing changes there.
pub fn hide_console_window(command: &mut Command) -> &mut Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    command
}

/// Appends `argument` to the child's command line exactly as written.
///
/// A Windows program receives one command-line string and parses it itself,
/// so a program with its own rules, such as `cmd.exe`, needs text that the
/// standard library has not quoted. A Unix program receives an argument
/// array, where every argument is already passed as written.
pub fn verbatim_argument(command: &mut Command, argument: impl AsRef<OsStr>) -> &mut Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.raw_arg(argument)
    }
    #[cfg(not(windows))]
    {
        command.arg(argument)
    }
}

/// The status of a process that exited normally with `code`.
pub fn exit_status_from_code(code: i32) -> ExitStatus {
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        ExitStatus::from_raw(code << 8)
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::ExitStatusExt;
        ExitStatus::from_raw(code as u32)
    }
}

#[cfg(test)]
mod tests {
    use super::exit_status_from_code;

    #[test]
    fn exit_codes_round_trip_through_a_status() {
        for code in [0, 1, 23, 128] {
            let status = exit_status_from_code(code);
            assert_eq!(status.code(), Some(code));
            assert_eq!(status.success(), code == 0);
        }
    }
}
