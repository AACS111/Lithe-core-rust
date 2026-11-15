//! Path text that follows platform rules.
//!
//! Core identifies workspace files by `/`-separated paths. These helpers
//! compare such paths and convert native path text the way the given
//! platform's file systems would.

use crate::Platform;
use std::borrow::Cow;
use std::path::Path;

/// Whether `path` is `ancestor` itself or lies below it.
///
/// Both paths are `/`-separated and relative to the same root, and `.` names
/// the root. Windows compares them case-insensitively, as its file systems
/// do; Unix compares them exactly.
pub fn relative_path_contains(ancestor: &str, path: &str, platform: Platform) -> bool {
    let folded;
    let (ancestor, path) = if platform.is_windows() {
        folded = (ancestor.to_lowercase(), path.to_lowercase());
        (folded.0.as_str(), folded.1.as_str())
    } else {
        (ancestor, path)
    };
    ancestor == "."
        || path == ancestor
        || path
            .strip_prefix(ancestor)
            .is_some_and(|suffix| suffix.starts_with('/'))
}

/// Writes every Windows `\` separator as `/`.
///
/// On Unix `\` is an ordinary file-name character, so the text is returned
/// unchanged.
pub fn slash_separated(path: &str, platform: Platform) -> Cow<'_, str> {
    if platform.is_windows() && path.contains('\\') {
        Cow::Owned(path.replace('\\', "/"))
    } else {
        Cow::Borrowed(path)
    }
}

/// Reads a path stored as bytes, such as a NUL-separated Git path list.
///
/// Unix paths are arbitrary bytes. Other platforms accept only UTF-8 and
/// return `None` for anything else.
pub fn native_path_from_bytes(bytes: &[u8]) -> Option<&Path> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        Some(Path::new(std::ffi::OsStr::from_bytes(bytes)))
    }
    #[cfg(not(unix))]
    {
        std::str::from_utf8(bytes).ok().map(Path::new)
    }
}

#[cfg(test)]
mod tests {
    use super::{native_path_from_bytes, relative_path_contains, slash_separated};
    use crate::Platform;

    #[test]
    fn the_root_contains_every_path() {
        for platform in [Platform::Unix, Platform::Windows] {
            assert!(relative_path_contains(".", "src/main.rs", platform));
            assert!(relative_path_contains(".", ".", platform));
        }
    }

    #[test]
    fn a_directory_contains_itself_and_its_descendants_only() {
        for platform in [Platform::Unix, Platform::Windows] {
            assert!(relative_path_contains("src", "src", platform));
            assert!(relative_path_contains("src", "src/main.rs", platform));
            assert!(relative_path_contains("src/app", "src/app/a.rs", platform));
            assert!(!relative_path_contains("src", "srcs/main.rs", platform));
            assert!(!relative_path_contains("src/main.rs", "src", platform));
            assert!(!relative_path_contains("", "src", platform));
        }
    }

    #[test]
    fn only_windows_ignores_case() {
        assert!(relative_path_contains(
            "Src",
            "src/Main.rs",
            Platform::Windows
        ));
        assert!(relative_path_contains(
            "ÄPFEL",
            "äpfel/a",
            Platform::Windows
        ));
        assert!(!relative_path_contains(
            "Src",
            "src/Main.rs",
            Platform::Unix
        ));
    }

    #[test]
    fn only_windows_treats_backslash_as_a_separator() {
        assert_eq!(
            slash_separated(r"C:\work\repo", Platform::Windows),
            "C:/work/repo"
        );
        assert_eq!(
            slash_separated(r"name\with\backslash", Platform::Unix),
            r"name\with\backslash"
        );
        assert_eq!(slash_separated("a/b", Platform::Windows), "a/b");
    }

    #[test]
    fn utf8_path_bytes_read_on_every_platform() {
        let path = native_path_from_bytes("src/äpfel.rs".as_bytes()).expect("UTF-8 path");
        assert_eq!(path.to_str(), Some("src/äpfel.rs"));
    }

    #[test]
    fn non_utf8_path_bytes_read_only_on_unix() {
        assert_eq!(native_path_from_bytes(b"src/\xff.rs").is_some(), cfg!(unix));
    }
}
