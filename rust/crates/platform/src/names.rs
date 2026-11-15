//! Windows file-name rules, checkable on every host.
//!
//! Windows reserves device names in every directory and strips or rejects
//! some characters, so a name that is ordinary on Unix can address a device,
//! open an alternate data stream, or be stored under a different name there.

/// Whether `component` names a Windows device such as `CON` or `COM1`.
///
/// Windows ignores everything from the first `.` and compares without case,
/// so `con.txt` and `Lpt1.log` address devices too. Superscript digits count
/// because Windows treats `COM¹` to `COM³` and `LPT¹` to `LPT³` as devices.
pub fn is_reserved_device_name(component: &str) -> bool {
    let base = component
        .split('.')
        .next()
        .unwrap_or_default()
        .to_ascii_uppercase();
    matches!(
        base.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
    ) || ["COM", "LPT"].iter().any(|prefix| {
        base.strip_prefix(prefix).is_some_and(|number| {
            matches!(
                number,
                "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
            )
        })
    })
}

/// Whether Windows stores a file called `component` under exactly that name.
///
/// Rejects device names, a trailing `.` or space (Windows strips both),
/// control characters, and `< > : " | ? *`; a `:` would also open an
/// alternate data stream of another file.
pub fn is_portable_windows_component(component: &str) -> bool {
    !component.ends_with(['.', ' '])
        && !component
            .chars()
            .any(|character| character.is_control() || "<>:\"|?*".contains(character))
        && !is_reserved_device_name(component)
}

#[cfg(test)]
mod tests {
    use super::{is_portable_windows_component, is_reserved_device_name};

    #[test]
    fn device_names_are_reserved_with_any_extension_and_case() {
        for name in [
            "CON",
            "con.txt",
            "Prn",
            "aux.log",
            "NUL",
            "CONIN$",
            "conout$.x",
            "COM1",
            "lpt9.txt",
            "COM¹",
            "LPT³.log",
        ] {
            assert!(is_reserved_device_name(name), "{name}");
        }
    }

    #[test]
    fn names_that_only_resemble_devices_are_not_reserved() {
        for name in [
            "CONSOLE", "COM0", "COM10", "LPT", "xCON", "nul_file", ".CON",
        ] {
            assert!(!is_reserved_device_name(name), "{name}");
        }
    }

    #[test]
    fn windows_rejects_names_it_would_store_differently() {
        for name in [
            "file:stream",
            "CON.txt",
            "LPT1",
            "trailing.",
            "space ",
            "a<b",
            "a>b",
            "a\"b",
            "a|b",
            "a?b",
            "a*b",
            "tab\tname",
        ] {
            assert!(!is_portable_windows_component(name), "{name}");
        }
    }

    #[test]
    fn ordinary_names_are_portable() {
        for name in [
            "App.java",
            "mvnw",
            ".gitignore",
            "name with spaces.txt",
            "äpfel",
        ] {
            assert!(is_portable_windows_component(name), "{name}");
        }
    }
}
