//! Starting the fixture program and waiting on it with deadlines.

use lithe_platform_tests::{ready_process_id, Mode};
use std::io::{BufRead, BufReader, ErrorKind, Read};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

/// The fixture program this package builds.
pub const FIXTURE: &str = env!("CARGO_BIN_EXE_lithe-platform-fixture");

/// Longest wait for one fixture event: generous on a loaded build machine,
/// and well below the fixture's own lifetime limit.
pub const DEADLINE: Duration = Duration::from_secs(10);

/// A command that starts the fixture in `mode`.
pub fn fixture(mode: Mode) -> Command {
    let mut command = Command::new(FIXTURE);
    command.arg(mode.name());
    command
}

#[derive(Debug)]
enum OutputEvent {
    Ready(u32),
    Closed,
    Failed(String),
}

/// Reads a fixture's standard output on its own thread.
pub struct OutputWatch {
    events: Receiver<OutputEvent>,
}

impl OutputWatch {
    /// Starts reading `output`.
    pub fn new(output: impl Read + Send + 'static) -> Self {
        let (sender, events) = mpsc::channel();
        std::thread::spawn(move || {
            let mut reader = BufReader::new(output);
            let mut line = Vec::new();
            let last = loop {
                line.clear();
                match reader.read_until(b'\n', &mut line) {
                    Ok(0) => break OutputEvent::Closed,
                    Ok(_) => {
                        if let Some(process_id) = ready_process_id(&String::from_utf8_lossy(&line))
                        {
                            let _ = sender.send(OutputEvent::Ready(process_id));
                        }
                    }
                    Err(error) if error.kind() == ErrorKind::Interrupted => {}
                    Err(error) => break OutputEvent::Failed(error.to_string()),
                }
            };
            let _ = sender.send(last);
        });
        Self { events }
    }

    /// Waits until a [`Mode::Hold`] fixture reports that it runs, and returns
    /// its process ID.
    pub fn ready(&self) -> u32 {
        match self.events.recv_timeout(DEADLINE) {
            Ok(OutputEvent::Ready(process_id)) => process_id,
            other => panic!("the fixture did not report that it runs: {other:?}"),
        }
    }

    /// Waits until the stream closes, which happens only once every process
    /// that shares it has exited. Panics with `failure` otherwise.
    pub fn wait_closed(&self, failure: &str) {
        let deadline = Instant::now() + DEADLINE;
        loop {
            match self
                .events
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            {
                Ok(OutputEvent::Closed) => return,
                Ok(OutputEvent::Ready(_)) => {}
                Ok(OutputEvent::Failed(error)) => panic!("{failure}: reading failed: {error}"),
                Err(error) => panic!("{failure}: {error}"),
            }
        }
    }
}

/// An empty directory under the system temporary directory, removed when
/// dropped.
pub struct TempDir(PathBuf);

impl TempDir {
    /// Creates a directory whose name starts with `prefix`, which may contain
    /// spaces.
    pub fn new(prefix: &str) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "{prefix} {} {}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        // A crashed earlier run with the same process ID may have left it.
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("temporary directory");
        Self(path)
    }

    /// The directory.
    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
