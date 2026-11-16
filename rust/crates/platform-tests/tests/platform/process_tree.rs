//! `lithe_git_host` runs and process trees on the real host.

use crate::support::{fixture, OutputWatch, DEADLINE};
use lithe_git_host::{run, Failure, ProcessTree};
use lithe_platform_tests::{Mode, FAILURE_ERROR, FAILURE_EXIT_CODE, FAILURE_OUTPUT, STREAM_LINE};
use std::process::{Child, ChildStdin, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

#[test]
fn incremental_output_can_cancel_before_the_child_exits() {
    let began = Instant::now();
    let started = AtomicBool::new(false);
    let received = AtomicBool::new(false);
    let mut command = fixture(Mode::Stream);
    let outcome = run(
        &mut command,
        None,
        || received.load(Ordering::Relaxed) || began.elapsed() > Duration::from_secs(2),
        || started.store(true, Ordering::Relaxed),
        |_, bytes| {
            assert!(started.load(Ordering::Relaxed));
            if bytes
                .windows(STREAM_LINE.len())
                .any(|window| window == STREAM_LINE.as_bytes())
            {
                received.store(true, Ordering::Relaxed);
            }
        },
    );
    assert!(
        received.load(Ordering::Relaxed),
        "deadline elapsed before fixture output"
    );
    assert!(matches!(outcome.failure, Some(Failure::Cancelled)));
    assert!(
        outcome.status.is_some(),
        "owned child must be reaped before return"
    );
}

#[test]
fn nonzero_exit_retains_both_streams() {
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut command = fixture(Mode::Failure);
    let outcome = run(
        &mut command,
        None,
        || Instant::now() > deadline,
        || {},
        |_, _| {},
    );
    assert!(outcome.failure.is_none());
    assert_eq!(
        outcome.status.and_then(|status| status.code()),
        Some(FAILURE_EXIT_CODE)
    );
    assert!(String::from_utf8_lossy(&outcome.stdout).contains(FAILURE_OUTPUT));
    assert!(String::from_utf8_lossy(&outcome.stderr).contains(FAILURE_ERROR));
}

/// A fixture of `mode` that leads its own [`ProcessTree`] and has reported
/// that it runs.
struct Tree {
    tree: ProcessTree,
    root: Child,
    output: OutputWatch,
    // Dropped last: closing it lets a surviving fixture exit on its own.
    _input: ChildStdin,
}

fn spawn_tree(mode: Mode) -> Tree {
    let mut command = fixture(mode);
    command.stdin(Stdio::piped()).stdout(Stdio::piped());
    ProcessTree::prepare(&mut command);
    let mut root = command.spawn().expect("spawn tree root");
    let input = root.stdin.take().expect("root input");
    let output = OutputWatch::new(root.stdout.take().expect("root output"));
    let tree = ProcessTree::adopt(&mut root).expect("adopt tree");
    let ready = output.ready();
    assert_eq!(
        ready == root.id(),
        mode == Mode::Hold,
        "a parent fixture reports its child"
    );
    Tree {
        tree,
        root,
        output,
        _input: input,
    }
}

/// Waits for `root` on another thread, as the owner of a real tree does
/// while it ends the tree.
fn reap_in_background(mut root: Child) -> Receiver<ExitStatus> {
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        if let Ok(status) = root.wait() {
            let _ = sender.send(status);
        }
    });
    receiver
}

#[test]
fn terminate_ends_the_root_and_its_descendants() {
    let Tree {
        tree,
        root,
        output,
        _input,
    } = spawn_tree(Mode::Parent);
    let root_exit = reap_in_background(root);

    tree.terminate(DEADLINE)
        .expect("every process of the tree ends");
    // The descendant shares the output pipe, so the stream closes only once
    // the descendant is gone too.
    output.wait_closed("ending the tree should end the descendant");
    root_exit.recv_timeout(DEADLINE).expect("root ended");
    // A second request after the tree is empty is harmless.
    tree.terminate(DEADLINE).expect("an empty tree terminates");
}

/// Why an owner reaps the root while it ends a tree: until then the root is a
/// zombie that still belongs to its Unix process group.
#[cfg(unix)]
#[test]
fn a_unix_tree_ends_only_once_its_root_is_reaped() {
    let Tree {
        tree,
        root,
        output,
        _input,
    } = spawn_tree(Mode::Hold);

    let unreaped = tree
        .terminate(Duration::from_millis(200))
        .expect_err("an unreaped root keeps the group alive");
    assert_eq!(unreaped.kind(), std::io::ErrorKind::TimedOut);
    // The root no longer runs: only its process-table entry is left.
    output.wait_closed("the killed root should close its output");

    let status = reap_in_background(root)
        .recv_timeout(DEADLINE)
        .expect("root reaped");
    assert!(!status.success());
    tree.terminate(DEADLINE)
        .expect("a reaped root no longer counts");
}

/// A group whose members have all exited but are not reaped yet is a normal
/// moment while a tree ends: the runtime a language server started can still
/// wait for its new parent to reap it after the root is gone. macOS refuses to
/// signal such a group (`EPERM`), and ending the tree must not report that as
/// a failure.
#[cfg(unix)]
#[test]
fn a_unix_tree_of_exited_unreaped_processes_ends_without_error() {
    let Tree {
        tree,
        root,
        output: _output,
        _input,
    } = spawn_tree(Mode::Hold);

    tree.kill().expect("a running tree is killed");
    wait_until_exited_unreaped(root.id());
    tree.kill()
        .expect("a tree whose processes all exited is killed again without error");

    let root_exit = reap_in_background(root);
    tree.terminate(DEADLINE)
        .expect("the tree ends once its exited root is reaped");
    let status = root_exit.recv_timeout(DEADLINE).expect("root reaped");
    assert!(!status.success());
}

/// Waits until the child `process_id` has exited and leaves it unreaped, so it
/// stays a zombie member of its process group until its `Child` is waited on.
#[cfg(unix)]
fn wait_until_exited_unreaped(process_id: u32) {
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || loop {
        // SAFETY: `info` is a valid out-parameter, and `WNOWAIT` leaves the
        // child for its `Child` owner to reap.
        let result = unsafe {
            let mut info: libc::siginfo_t = std::mem::zeroed();
            libc::waitid(
                libc::P_PID,
                process_id as libc::id_t,
                &mut info,
                libc::WEXITED | libc::WNOWAIT,
            )
        };
        let error = std::io::Error::last_os_error();
        if result != 0 && error.kind() == std::io::ErrorKind::Interrupted {
            continue;
        }
        let _ = sender.send(if result == 0 { Ok(()) } else { Err(error) });
        break;
    });
    receiver
        .recv_timeout(DEADLINE)
        .expect("the killed root exits")
        .expect("wait for the root without reaping it");
}

#[cfg(windows)]
#[test]
fn dropping_a_process_tree_ends_it_like_a_crashed_host() {
    // The kernel closes the job handle when the host dies; dropping the tree
    // closes it the same way, without an explicit terminate.
    let Tree {
        tree,
        root,
        output,
        _input,
    } = spawn_tree(Mode::Parent);
    let root_exit = reap_in_background(root);

    drop(tree);
    root_exit
        .recv_timeout(DEADLINE)
        .expect("closing the job ends the root");
    output.wait_closed("closing the job ends the descendant");
}
