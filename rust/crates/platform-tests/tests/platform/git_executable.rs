//! Git executable resolution on the host's file system.

use crate::support::TempDir;
use lithe_git_host::configuration::executable;

// Regression: a backslash launch path makes some Windows hosts capture empty
// Git output, so the resolved executable must reach the process spawner with
// forward slashes.
#[cfg(windows)]
#[test]
fn selected_executable_is_resolved_with_forward_slashes() {
    let directory = TempDir::new("lithe git launch path");
    let file = directory.path().join("git.exe");
    std::fs::write(&file, b"").expect("placeholder executable");

    let resolved = executable(file.to_str())
        .expect("a selected existing file is used")
        .to_string_lossy()
        .into_owned();

    assert!(!resolved.contains('\\'), "{resolved}");
    assert!(resolved.ends_with("/git.exe"), "{resolved}");
}

#[cfg(unix)]
#[test]
fn a_selected_file_is_used_once_it_has_an_execute_bit() {
    use std::os::unix::fs::PermissionsExt;

    let directory = TempDir::new("lithe git launch path");
    let file = directory.path().join("git");
    std::fs::write(&file, b"#!/bin/sh\n").expect("placeholder executable");
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644)).expect("chmod");
    assert_eq!(executable(file.to_str()), None);

    lithe_platform::fs::add_execute_bits(&file).expect("add execute bits");

    assert_eq!(executable(file.to_str()), Some(file));
}
