//! File permissions that only Unix stores.
//!
//! Unix keeps owner, group and other permission bits on every file. Windows
//! uses access-control lists that a new file inherits from its directory, and
//! decides whether a file runs by its extension. These helpers apply or read
//! the Unix bits and do nothing on Windows, so callers state their intent once
//! instead of repeating a `cfg` branch.

use std::fs::{DirBuilder, Metadata, OpenOptions};
use std::io;
use std::path::Path;

/// Owner read and write: the Unix mode of a private file.
pub const OWNER_ONLY_FILE_MODE: u32 = 0o600;

/// Owner read, write and search: the Unix mode of a private directory.
pub const OWNER_ONLY_DIRECTORY_MODE: u32 = 0o700;

/// Execute permission for owner, group and others.
pub const EXECUTE_BITS: u32 = 0o111;

/// Makes a file that `options` creates readable and writable only by its
/// owner. An existing file keeps its permissions.
pub fn owner_only_file(options: &mut OpenOptions) -> &mut OpenOptions {
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(OWNER_ONLY_FILE_MODE);
    }
    options
}

/// Applies the Unix `mode` to each directory that `builder` creates.
pub fn directory_mode(builder: &mut DirBuilder, mode: u32) -> &mut DirBuilder {
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(mode);
    }
    #[cfg(not(unix))]
    let _ = mode;
    builder
}

/// Restricts the existing directory at `path` to its owner.
pub fn restrict_directory_to_owner(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(
            path,
            std::fs::Permissions::from_mode(OWNER_ONLY_DIRECTORY_MODE),
        )
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        Ok(())
    }
}

/// The Unix mode of `metadata`, including its file-type bits, or `None` on a
/// platform that stores no mode.
pub fn unix_mode(metadata: &Metadata) -> Option<u32> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        Some(metadata.permissions().mode())
    }
    #[cfg(not(unix))]
    {
        let _ = metadata;
        None
    }
}

/// Whether a Unix `mode` lets anyone execute the file.
pub const fn mode_is_executable(mode: u32) -> bool {
    mode & EXECUTE_BITS != 0
}

/// Whether `metadata` carries a Unix execute bit. Always `false` where the
/// platform stores no mode.
pub fn has_execute_bit(metadata: &Metadata) -> bool {
    unix_mode(metadata).is_some_and(mode_is_executable)
}

/// Whether the host could start the file `metadata` describes as a program:
/// a regular file that, on Unix, also has an execute bit.
pub fn is_launchable_file(metadata: &Metadata) -> bool {
    metadata.is_file() && unix_mode(metadata).is_none_or(mode_is_executable)
}

/// Adds [`EXECUTE_BITS`] to the file at `path`.
pub fn add_execute_bits(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = std::fs::metadata(path)?.permissions();
        permissions.set_mode(permissions.mode() | EXECUTE_BITS);
        std::fs::set_permissions(path, permissions)
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::mode_is_executable;

    #[test]
    fn any_execute_bit_makes_a_mode_executable() {
        for mode in [0o100, 0o010, 0o001, 0o755, 0o100_755] {
            assert!(mode_is_executable(mode), "{mode:o}");
        }
        for mode in [0o644, 0o600, 0o100_644] {
            assert!(!mode_is_executable(mode), "{mode:o}");
        }
    }
}
