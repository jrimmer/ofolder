//! Talking to a local socket without ever waiting for it for good.
//!
//! The compositor and the pickers answer in microseconds when they work. A plain `connect` to a
//! program that stopped accepting blocks the caller once its queue is full, and a plain `read`
//! blocks while it stays silent; the caller may be the docking thread, so every step here has
//! a deadline and gives up with an error the caller turns into its fallback.

use std::io::{self, Read, Write};
use std::os::fd::{FromRawFd, OwnedFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::thread::sleep;
use std::time::{Duration, Instant};

fn timed_out(what: &str) -> io::Error {
    io::Error::new(io::ErrorKind::TimedOut, format!("{what} timed out"))
}

/// Connects to the socket at `path`, giving up at `limit`.
pub fn connect_within(path: &Path, limit: Duration) -> io::Result<UnixStream> {
    let bytes = path.as_os_str().as_bytes();
    // SAFETY: sockaddr_un is plain data and all-zero is a valid value for it.
    let mut address: libc::sockaddr_un = unsafe { std::mem::zeroed() };
    if bytes.len() >= address.sun_path.len() || bytes.contains(&0) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "the socket path does not fit",
        ));
    }
    address.sun_family = libc::AF_UNIX as libc::sa_family_t;
    for (slot, byte) in address.sun_path.iter_mut().zip(bytes) {
        *slot = *byte as libc::c_char;
    }
    // SAFETY: plain system call; the result is checked before it is used.
    let raw = unsafe {
        libc::socket(
            libc::AF_UNIX,
            libc::SOCK_STREAM | libc::SOCK_CLOEXEC | libc::SOCK_NONBLOCK,
            0,
        )
    };
    if raw < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: `raw` is a socket this function just made and nothing else owns.
    let owned = unsafe { OwnedFd::from_raw_fd(raw) };
    let started = Instant::now();
    loop {
        // SAFETY: `address` is a valid sockaddr_un and the length is its size.
        let rc = unsafe {
            libc::connect(
                raw,
                (&raw const address).cast::<libc::sockaddr>(),
                std::mem::size_of::<libc::sockaddr_un>() as libc::socklen_t,
            )
        };
        if rc == 0 {
            break;
        }
        let error = io::Error::last_os_error();
        // A full queue answers "try again" on a non-blocking socket; anything else is final.
        if error.raw_os_error() != Some(libc::EAGAIN) || started.elapsed() >= limit {
            return Err(if started.elapsed() >= limit {
                timed_out("connecting")
            } else {
                error
            });
        }
        sleep(Duration::from_millis(2));
    }
    let stream = UnixStream::from(owned);
    stream.set_nonblocking(false)?;
    Ok(stream)
}

/// Reads until the peer closes the stream or `stop` says a line is complete, within `limit` as a
/// whole and never past `cap` bytes.
fn read_within(
    stream: &mut UnixStream,
    started: Instant,
    limit: Duration,
    cap: usize,
    stop: impl Fn(&[u8]) -> bool,
) -> io::Result<Vec<u8>> {
    let mut data = Vec::new();
    let mut chunk = [0u8; 8192];
    loop {
        let left = limit
            .checked_sub(started.elapsed())
            .filter(|d| !d.is_zero())
            .ok_or_else(|| timed_out("reading"))?;
        stream.set_read_timeout(Some(left))?;
        match stream.read(&mut chunk) {
            Ok(0) => return Ok(data),
            Ok(n) => {
                data.extend_from_slice(&chunk[..n]);
                if data.len() > cap {
                    return Err(io::Error::other("the reply is larger than allowed"));
                }
                if stop(&data) {
                    return Ok(data);
                }
            }
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e)
                if matches!(
                    e.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                ) =>
            {
                return Err(timed_out("reading"));
            }
            Err(e) => return Err(e),
        }
    }
}

/// Sends `request` and returns everything the peer sends before it closes the stream. For a
/// peer that answers once and hangs up, as Hyprland's request socket does.
pub fn request_until_close(
    path: &Path,
    request: &[u8],
    limit: Duration,
    cap: usize,
) -> io::Result<Vec<u8>> {
    let started = Instant::now();
    let mut stream = connect_within(path, limit)?;
    stream.set_write_timeout(Some(limit))?;
    stream.write_all(request)?;
    read_within(&mut stream, started, limit, cap, |_| false)
}

/// Sends `request` (a line) and returns the first line of the answer, without its newline.
pub fn request_line(
    path: &Path,
    request: &[u8],
    limit: Duration,
    cap: usize,
) -> io::Result<Vec<u8>> {
    let started = Instant::now();
    let mut stream = connect_within(path, limit)?;
    stream.set_write_timeout(Some(limit))?;
    stream.write_all(request)?;
    let mut data = read_within(&mut stream, started, limit, cap, |d| d.contains(&b'\n'))?;
    match data.iter().position(|b| *b == b'\n') {
        Some(end) => {
            data.truncate(end);
            Ok(data)
        }
        None => Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "the peer closed before ending its line",
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::net::UnixListener;

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("ofolder-sockio-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("s.sock")
    }

    #[test]
    fn an_answer_is_read_until_the_peer_hangs_up() {
        let path = scratch("close");
        let listener = UnixListener::bind(&path).unwrap();
        let server = std::thread::spawn(move || {
            let (mut c, _) = listener.accept().unwrap();
            let mut got = [0u8; 4];
            c.read_exact(&mut got).unwrap();
            c.write_all(b"hello ").unwrap();
            c.write_all(&got).unwrap();
        });
        let reply = request_until_close(&path, b"ping", Duration::from_secs(2), 1024).unwrap();
        assert_eq!(reply, b"hello ping");
        server.join().unwrap();
    }

    #[test]
    fn a_silent_peer_times_out_instead_of_blocking() {
        let path = scratch("silent");
        let listener = UnixListener::bind(&path).unwrap();
        let started = Instant::now();
        let error = request_until_close(&path, b"x", Duration::from_millis(150), 1024).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        assert!(started.elapsed() < Duration::from_secs(2));
        drop(listener);
    }

    #[test]
    fn a_full_queue_times_out_instead_of_blocking() {
        let path = scratch("full");
        let listener = UnixListener::bind(&path).unwrap();
        // SAFETY: listening again on a socket this test owns only changes its queue length.
        unsafe { libc::listen(std::os::fd::AsRawFd::as_raw_fd(&listener), 1) };
        // Nobody accepts, so connections pile up until the queue is full.
        let mut held = Vec::new();
        let started = Instant::now();
        let mut failed = false;
        for _ in 0..50 {
            match connect_within(&path, Duration::from_millis(100)) {
                Ok(s) => held.push(s),
                Err(e) => {
                    assert_eq!(e.kind(), io::ErrorKind::TimedOut);
                    failed = true;
                    break;
                }
            }
        }
        assert!(failed, "the queue never filled");
        assert!(started.elapsed() < Duration::from_secs(10));
        drop(listener);
    }

    #[test]
    fn a_missing_socket_is_an_error() {
        let path = scratch("missing").with_file_name("nothing.sock");
        assert!(connect_within(&path, Duration::from_millis(100)).is_err());
    }

    #[test]
    fn a_line_is_returned_without_waiting_for_the_peer_to_close() {
        let path = scratch("line");
        let listener = UnixListener::bind(&path).unwrap();
        let server = std::thread::spawn(move || {
            let (mut c, _) = listener.accept().unwrap();
            c.write_all(b"{\"ok\":true}\nmore").unwrap();
            std::thread::sleep(Duration::from_millis(500));
        });
        let started = Instant::now();
        let line = request_line(&path, b"q\n", Duration::from_secs(2), 1024).unwrap();
        assert_eq!(line, b"{\"ok\":true}");
        assert!(started.elapsed() < Duration::from_millis(400));
        server.join().unwrap();
    }

    #[test]
    fn a_reply_over_the_cap_is_refused() {
        let path = scratch("cap");
        let listener = UnixListener::bind(&path).unwrap();
        let server = std::thread::spawn(move || {
            let (mut c, _) = listener.accept().unwrap();
            c.write_all(&vec![b'a'; 5000]).ok();
        });
        assert!(request_until_close(&path, b"x", Duration::from_secs(2), 1000).is_err());
        server.join().unwrap();
    }
}
