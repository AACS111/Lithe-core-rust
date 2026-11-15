//! Native executable resolution and per-child Git environment configuration.
use lithe_platform::fs::is_launchable_file;
use lithe_platform::paths::slash_separated;
use lithe_platform::Platform;
use std::path::PathBuf;
use std::process::Command;

/// Resolves a user-selected executable or the first runnable Git on PATH.
pub fn executable(selected: Option<&str>) -> Option<PathBuf> {
    let platform = Platform::HOST;
    let candidates = match selected {
        Some(path) => vec![PathBuf::from(path)],
        None => {
            let name = platform.executable_name("git");
            std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
                .map(|directory| directory.join(&name))
                .collect()
        }
    };
    candidates
        .into_iter()
        .find(|path| {
            path.metadata()
                .is_ok_and(|metadata| is_launchable_file(&metadata))
        })
        .map(|path| launch_path(path, platform))
}

/// Some Windows hosts (disk-encryption or endpoint-security filters) make a Git started through a
/// backslash path print nothing to a captured pipe, while the same executable started through a
/// forward-slash path behaves normally. Windows accepts both separators, so launch with `/`.
/// Verbatim (`\\?\`) and UNC (`\\server`) paths keep their native form because `/` is not valid
/// there.
fn launch_path(path: PathBuf, platform: Platform) -> PathBuf {
    match path.to_str() {
        Some(text) if !text.starts_with(r"\\") => {
            PathBuf::from(slash_separated(text, platform).into_owned())
        }
        _ => path,
    }
}

/// Preserves inherited config entries and appends named invocation overrides.
/// No global process environment or config file is changed.
pub fn configure(command: &mut Command, values: &[(String, String)]) -> std::io::Result<()> {
    let inherited = inherited_count(std::env::var("GIT_CONFIG_COUNT").ok().as_deref())?;
    for (offset, (key, value)) in values.iter().enumerate() {
        command
            .env(format!("GIT_CONFIG_KEY_{}", inherited + offset), key)
            .env(format!("GIT_CONFIG_VALUE_{}", inherited + offset), value);
    }
    command
        .env("GIT_CONFIG_COUNT", (inherited + values.len()).to_string())
        .env("GIT_PAGER", "cat")
        .env("LC_ALL", "C")
        .env("GIT_TERMINAL_PROMPT", "0");
    Ok(())
}

fn inherited_count(value: Option<&str>) -> std::io::Result<usize> {
    match value {
        None => Ok(0),
        Some(value) => value
            .parse::<usize>()
            .ok()
            .filter(|count| *count <= 4096)
            .ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "Invalid inherited Git configuration count",
                )
            }),
    }
}

/// Bounded native version probe cached by executable identity; replacement invalidates the cache.
pub fn version(
    program: &std::ffi::OsStr,
    mut cancelled: impl FnMut() -> bool,
) -> std::io::Result<String> {
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};
    use std::time::{Duration, Instant, SystemTime};
    type Cache = HashMap<PathBuf, (u64, Option<SystemTime>, String)>;
    static CACHE: OnceLock<Mutex<Cache>> = OnceLock::new();
    let path = PathBuf::from(program);
    let metadata = path.metadata()?;
    let identity = (metadata.len(), metadata.modified().ok());
    let cache = CACHE.get_or_init(Default::default);
    if let Some((size, modified, version)) = cache
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
        .get(&path)
    {
        if (*size, *modified) == identity {
            return Ok(version.clone());
        }
    }
    let started = Instant::now();
    let output = crate::run(
        Command::new(program).arg("--version"),
        None,
        || cancelled() || started.elapsed() >= Duration::from_secs(5),
        || {},
        |_, _| {},
    );
    if output.failure.is_some() || !output.status.is_some_and(|status| status.success()) {
        return Err(std::io::Error::other(
            "Could not determine Git version within the operation deadline",
        ));
    }
    let version = String::from_utf8(output.stdout)
        .map_err(|_| std::io::Error::other("Git version is not UTF-8"))?
        .trim()
        .to_owned();
    // An empty report is a broken probe, not an old Git; keep it out of the cache and name it.
    if version.is_empty() {
        return Err(std::io::Error::other(
            "Git exited successfully but printed no version output",
        ));
    }
    let mut cache = cache.lock().unwrap_or_else(|poison| poison.into_inner());
    if cache.len() >= 16 {
        cache.clear();
    }
    cache.insert(path, (identity.0, identity.1, version.clone()));
    Ok(version)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn inherited_configuration_rejects_invalid_and_unbounded_counts() {
        assert_eq!(inherited_count(None).unwrap(), 0);
        assert_eq!(inherited_count(Some("2")).unwrap(), 2);
        for value in ["", "-1", "bad", "4097", "18446744073709551615"] {
            assert!(inherited_count(Some(value)).is_err());
        }
    }

    #[test]
    fn windows_launch_paths_use_forward_slashes_except_unc_and_verbatim() {
        let launch = |path: &str| launch_path(PathBuf::from(path), Platform::Windows);
        assert_eq!(
            launch(r"D:\Program Files\Git\cmd\git.exe"),
            PathBuf::from("D:/Program Files/Git/cmd/git.exe")
        );
        assert_eq!(
            launch("D:/Git/cmd/git.exe"),
            PathBuf::from("D:/Git/cmd/git.exe")
        );
        for native in [r"\\server\share\git.exe", r"\\?\D:\Git\cmd\git.exe"] {
            assert_eq!(launch(native), PathBuf::from(native));
        }
    }

    #[test]
    fn unix_launch_paths_keep_backslashes_as_name_characters() {
        let path = PathBuf::from(r"/opt/odd\name/git");
        assert_eq!(launch_path(path.clone(), Platform::Unix), path);
    }
}
