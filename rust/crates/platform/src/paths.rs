//! Path text that follows platform rules.
//!
//! Core identifies workspace files by `/`-separated paths. These helpers
//! compare such paths, convert native path text the way the given platform's
//! file systems would, and read the native absolute paths and path lists a
//! tool prints.

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

/// Separator between the entries of a path list such as `PATH`.
pub const fn path_list_separator(platform: Platform) -> char {
    match platform {
        Platform::Windows => ';',
        Platform::Unix => ':',
    }
}

/// Whether native path text is absolute, judged from the text alone.
///
/// Unix paths start with `/`. Windows paths start with a drive letter and a
/// separator (`C:\`, `C:/`) or are UNC paths (`\\server\share`); a bare `C:`
/// or a rooted `\dir` still depends on the current drive and is not absolute.
pub fn is_absolute_path(path: &str, platform: Platform) -> bool {
    match platform {
        Platform::Unix => path.starts_with('/'),
        Platform::Windows => {
            let bytes = path.as_bytes();
            let drive = bytes.len() >= 3
                && bytes[0].is_ascii_alphabetic()
                && bytes[1] == b':'
                && matches!(bytes[2], b'\\' | b'/');
            drive || path.starts_with(r"\\") || path.starts_with("//")
        }
    }
}

/// Appends a `/`-separated relative path to a native directory with the
/// platform's separator, dropping separators that already end `directory`.
pub fn join_native(directory: &str, relative: &str, platform: Platform) -> String {
    let is_separator =
        |character: char| character == '/' || (platform.is_windows() && character == '\\');
    let trimmed = directory.trim_end_matches(is_separator);
    let (separator, relative) = if platform.is_windows() {
        ('\\', relative.replace('/', "\\"))
    } else {
        ('/', relative.to_owned())
    };
    // The root directory is only separators; keep it as it is.
    if trimmed.is_empty() {
        return format!("{directory}{relative}");
    }
    format!("{trimmed}{separator}{relative}")
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
    use super::{
        is_absolute_path, join_native, native_path_from_bytes, path_list_separator,
        relative_path_contains, slash_separated,
    };
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
    fn path_lists_use_the_platform_separator() {
        assert_eq!(path_list_separator(Platform::Unix), ':');
        assert_eq!(path_list_separator(Platform::Windows), ';');
    }

    #[test]
    fn absolute_paths_follow_the_platform() {
        assert!(is_absolute_path("/home/me/go", Platform::Unix));
        assert!(!is_absolute_path("go/bin", Platform::Unix));
        assert!(!is_absolute_path(r"C:\Users\me", Platform::Unix));
        assert!(is_absolute_path(r"C:\Users\me\go", Platform::Windows));
        assert!(is_absolute_path("C:/Users/me/go", Platform::Windows));
        assert!(is_absolute_path(r"\\server\share\go", Platform::Windows));
        assert!(!is_absolute_path("C:", Platform::Windows));
        assert!(!is_absolute_path("C:go", Platform::Windows));
        assert!(!is_absolute_path(r"\go", Platform::Windows));
        assert!(!is_absolute_path("go", Platform::Windows));
    }

    #[test]
    fn joined_paths_use_the_platform_separator() {
        assert_eq!(
            join_native("/home/me/go", "bin", Platform::Unix),
            "/home/me/go/bin"
        );
        assert_eq!(
            join_native("/home/me/go/", "a/b", Platform::Unix),
            "/home/me/go/a/b"
        );
        assert_eq!(join_native("/", "bin", Platform::Unix), "/bin");
        assert_eq!(
            join_native(r"C:\Users\me\go\", "a/b", Platform::Windows),
            r"C:\Users\me\go\a\b"
        );
        assert_eq!(
            join_native("C:/Users/me/go", "bin", Platform::Windows),
            r"C:/Users/me/go\bin"
        );
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
