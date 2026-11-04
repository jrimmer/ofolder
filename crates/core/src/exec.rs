//! Running another program with a time limit.
//!
//! The compositor and the other helpers ofolder runs answer in milliseconds when they work. When one
//! wedges, a plain `output()` or `wait()` blocks the thread that asked, and that thread may be the
//! one that draws the rails. Every call here has a deadline instead: the program is killed when it is
//! reached, and reaped, so nothing is left running behind a dialog.

use std::io::{self, Read, Write};
use std::process::{Child, Command, ExitStatus, Output, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::{Duration, Instant};

/// How often a running program is looked at while the deadline is waited for.
const POLL: Duration = Duration::from_millis(5);

#[derive(Debug, thiserror::Error)]
pub enum ExecError {
    #[error("{program} could not be started: {source}")]
    Start { program: String, source: io::Error },
    #[error("{program} did not finish within {limit:?}, so it was stopped")]
    TimedOut { program: String, limit: Duration },
    #[error("{program} could not be waited for: {source}")]
    Wait { program: String, source: io::Error },
}

/// Runs `command` with no input and returns its status and both output streams, or gives up at
/// `limit` and kills the program.
pub fn run_with_deadline(command: &mut Command, limit: Duration) -> Result<Output, ExecError> {
    run_with_deadline_stdin(command, None, limit)
}

/// [`run_with_deadline`] with `stdin` written to the program first, when there is any.
///
/// Both output streams are read while the program runs, so output larger than a pipe's buffer
/// cannot stall it. The input is written on its own thread for the same reason.
pub fn run_with_deadline_stdin(
    command: &mut Command,
    stdin: Option<&[u8]>,
    limit: Duration,
) -> Result<Output, ExecError> {
    let program = program_name(command);
    let started = Instant::now();
    command
        .stdin(if stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = spawn(command, &program)?;
    if let (Some(mut pipe), Some(bytes)) = (child.stdin.take(), stdin) {
        let bytes = bytes.to_vec();
        // A program that exits without reading its input closes the pipe, and the error is the
        // program's to report through its status, so it is not reported here.
        thread::spawn(move || {
            pipe.write_all(&bytes).ok();
        });
    }
    let stdout = read_to_end_in_background(child.stdout.take());
    let stderr = read_to_end_in_background(child.stderr.take());
    let status = wait_within(&mut child, &program, limit)?;
    // The streams end when the program closes them. A program that handed them to a process of its
    // own could keep them open, so they are waited for only within the same limit.
    let stdout = collect_within(
        stdout,
        &program,
        limit.saturating_sub(started.elapsed()),
        limit,
    )?;
    let stderr = collect_within(
        stderr,
        &program,
        limit.saturating_sub(started.elapsed()),
        limit,
    )?;
    Ok(Output {
        status,
        stdout,
        stderr,
    })
}

/// Writes `input` to the program's stdin and waits for it, with both output streams discarded.
///
/// For a program that serves its input from a process of its own (`wl-copy` does): the helper
/// process may keep a pipe open for as long as it serves, so the output is not waited for, and only
/// the program's own exit status is.
pub fn feed_with_deadline(
    command: &mut Command,
    input: &[u8],
    limit: Duration,
) -> Result<ExitStatus, ExecError> {
    let program = program_name(command);
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let mut child = spawn(command, &program)?;
    if let Some(mut pipe) = child.stdin.take() {
        let bytes = input.to_vec();
        thread::spawn(move || {
            pipe.write_all(&bytes).ok();
        });
    }
    wait_within(&mut child, &program, limit)
}

fn program_name(command: &Command) -> String {
    command.get_program().to_string_lossy().into_owned()
}

fn spawn(command: &mut Command, program: &str) -> Result<Child, ExecError> {
    command.spawn().map_err(|source| ExecError::Start {
        program: program.to_owned(),
        source,
    })
}

/// Waits for the program to exit. At the deadline it is killed and reaped, and the error says so.
fn wait_within(child: &mut Child, program: &str, limit: Duration) -> Result<ExitStatus, ExecError> {
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Ok(status),
            Ok(None) if started.elapsed() >= limit => {
                // A kill that fails because the program exited a moment ago does no harm: the wait
                // below reaps it either way, and that is what keeps it from being left behind.
                child.kill().ok();
                child.wait().ok();
                return Err(ExecError::TimedOut {
                    program: program.to_owned(),
                    limit,
                });
            }
            Ok(None) => thread::sleep(POLL),
            Err(source) => {
                child.kill().ok();
                child.wait().ok();
                return Err(ExecError::Wait {
                    program: program.to_owned(),
                    source,
                });
            }
        }
    }
}

/// Reads a stream to its end on a thread, and sends what it read once. The thread is never joined,
/// so a stream that stays open cannot hold the caller.
fn read_to_end_in_background(
    pipe: Option<impl Read + Send + 'static>,
) -> Option<Receiver<Vec<u8>>> {
    let mut pipe = pipe?;
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let mut bytes = Vec::new();
        // A read error ends the output the same way the end of the stream does: with what came.
        pipe.read_to_end(&mut bytes).ok();
        sender.send(bytes).ok();
    });
    Some(receiver)
}

/// The output of a stream, or nothing when the stream was not captured. It must arrive within the
/// part of `remaining` that is left, which is the caller's whole `limit` less the time already used.
fn collect_within(
    stream: Option<Receiver<Vec<u8>>>,
    program: &str,
    remaining: Duration,
    limit: Duration,
) -> Result<Vec<u8>, ExecError> {
    let Some(receiver) = stream else {
        return Ok(Vec::new());
    };
    receiver
        .recv_timeout(remaining)
        .map_err(|_| ExecError::TimedOut {
            program: program.to_owned(),
            limit,
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A directory under the system temporary folder, removed on drop.
    struct Scratch(std::path::PathBuf);

    impl Scratch {
        fn new(label: &str) -> Self {
            let dir =
                std::env::temp_dir().join(format!("ofolder-exec-{label}-{}", std::process::id()));
            std::fs::create_dir_all(&dir).expect("the temporary folder can be created");
            Self(dir)
        }

        fn path(&self, name: &str) -> std::path::PathBuf {
            self.0.join(name)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.0).ok();
        }
    }

    fn shell(script: &str) -> Command {
        let mut command = Command::new("sh");
        command.args(["-c", script]);
        command
    }

    #[test]
    fn a_program_that_hangs_is_stopped_at_the_deadline_and_no_longer_runs() {
        let scratch = Scratch::new("hang");
        let pid_file = scratch.path("pid");
        // `exec` makes the shell itself the sleeper, so the pid the script records is the process
        // that must be gone afterwards.
        let mut command = Command::new("sh");
        command
            .args(["-c", "echo $$ > \"$0\"; exec sleep 30"])
            .arg(&pid_file);
        let limit = Duration::from_secs(1);
        let started = Instant::now();
        let result = run_with_deadline(&mut command, limit);
        assert!(
            matches!(result, Err(ExecError::TimedOut { .. })),
            "{result:?}"
        );
        assert!(started.elapsed() < limit + Duration::from_secs(1));
        let pid = std::fs::read_to_string(&pid_file)
            .expect("the script recorded its pid before it was stopped")
            .trim()
            .to_owned();
        assert!(
            !std::path::Path::new(&format!("/proc/{pid}")).exists(),
            "process {pid} is still there after its deadline"
        );
    }

    #[test]
    fn a_fast_program_returns_its_output_and_status() {
        let output = run_with_deadline(
            &mut shell("printf out; printf err >&2; exit 0"),
            Duration::from_secs(5),
        )
        .expect("a fast program finishes");
        assert!(output.status.success());
        assert_eq!(output.stdout, b"out");
        assert_eq!(output.stderr, b"err");
    }

    #[test]
    fn a_failing_program_reports_its_status_and_is_not_an_error() {
        let output = run_with_deadline(&mut shell("exit 3"), Duration::from_secs(5))
            .expect("a program that exits is not a timeout");
        assert_eq!(output.status.code(), Some(3));
    }

    #[test]
    fn two_hundred_kibibytes_on_both_streams_at_once_do_not_deadlock() {
        let output = run_with_deadline(
            &mut shell("head -c 204800 /dev/zero >&2 & head -c 204800 /dev/zero; wait"),
            Duration::from_secs(10),
        )
        .expect("output larger than a pipe buffer is read while the program runs");
        assert_eq!(output.stdout.len(), 204_800);
        assert_eq!(output.stderr.len(), 204_800);
    }

    #[test]
    fn the_input_is_delivered_to_the_program_even_when_it_is_large() {
        let input = vec![b'x'; 204_800];
        let output = run_with_deadline_stdin(
            &mut Command::new("cat"),
            Some(&input),
            Duration::from_secs(10),
        )
        .expect("cat echoes its input");
        assert_eq!(output.stdout, input);
    }

    #[test]
    fn a_program_given_no_input_reads_an_empty_stdin() {
        let output = run_with_deadline(&mut Command::new("cat"), Duration::from_secs(5))
            .expect("cat with no input ends");
        assert!(output.stdout.is_empty());
    }

    #[test]
    fn a_program_that_does_not_exist_is_a_start_error_that_names_it() {
        let error = run_with_deadline(
            &mut Command::new("ofolder-no-such-program-for-exec-test"),
            Duration::from_secs(5),
        )
        .expect_err("a missing program cannot run");
        assert!(matches!(error, ExecError::Start { .. }), "{error:?}");
        assert!(
            error
                .to_string()
                .contains("ofolder-no-such-program-for-exec-test")
        );
    }

    #[test]
    fn feed_delivers_its_input_and_returns_the_status() {
        let scratch = Scratch::new("feed");
        let target = scratch.path("received");
        let status = feed_with_deadline(
            Command::new("sh").args(["-c", "cat > \"$0\""]).arg(&target),
            b"text on stdin",
            Duration::from_secs(5),
        )
        .expect("the program finishes");
        assert!(status.success());
        assert_eq!(
            std::fs::read(&target).expect("the program wrote it"),
            b"text on stdin"
        );
    }

    #[test]
    fn feed_stops_a_program_that_hangs_at_the_deadline() {
        let started = Instant::now();
        let result = feed_with_deadline(
            &mut shell("exec sleep 30"),
            b"ignored",
            Duration::from_millis(300),
        );
        assert!(
            matches!(result, Err(ExecError::TimedOut { .. })),
            "{result:?}"
        );
        assert!(started.elapsed() < Duration::from_secs(2));
    }
}
