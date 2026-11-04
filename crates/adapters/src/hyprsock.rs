//! The compositor, through Hyprland's own sockets instead of the `hyprctl` command.
//!
//! Reads that the docking loop makes several times a second (windows, monitors, the active window,
//! the pointer) go to the request socket, which needs no new process. The event socket tells the
//! loop when a window opens, closes, changes workspace, floats or takes focus, so it need not
//! look on a timer to notice. Every other call, and every read whose socket fails, goes to
//! [`Hyprctl`], which behaves exactly as before. Hyprland's event text is data: only the event
//! name is compared with a fixed list, and nothing in it is ever used as a window address.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use ofolder_core::hypr::{Client, Monitor, parse_clients, parse_monitors};
use ofolder_core::keybind::Bind;
use ofolder_core::shortcuts::Combo;
use ofolder_core::system::{Compositor, SystemError};
use ofolder_core::wake::Wake;

use crate::hyprctl::{Hyprctl, parse_active, parse_cursor};
use crate::sockio::{connect_within, request_until_close};

/// Hyprland answers in well under a millisecond; a socket still silent after this is wedged, and
/// the read is made by `hyprctl` instead.
const REQUEST_LIMIT: Duration = Duration::from_millis(500);
/// The largest reply read from the request socket.
const REPLY_CAP: usize = 16 * 1024 * 1024;
/// After a failed read the socket is left alone for this long, so a dead socket costs one failed
/// try every few seconds and not one per read.
const REST_AFTER_FAILURE: Duration = Duration::from_secs(5);
/// The longest event line kept; a longer one is dropped whole.
const LINE_CAP: usize = 64 * 1024;
/// Between two tries to reach the event socket again.
const RECONNECT: Duration = Duration::from_secs(2);

/// The events that mean the set of windows, or which one is active, or where one is, may have
/// changed. Anything else is ignored.
const WINDOW_EVENTS: &[&str] = &[
    "openwindow",
    "closewindow",
    "movewindow",
    "movewindowv2",
    "activewindow",
    "activewindowv2",
    "changefloatingmode",
    "windowtitle",
    "windowtitlev2",
    "workspace",
    "workspacev2",
    "moveworkspace",
    "moveworkspacev2",
    "focusedmon",
    "focusedmonv2",
    "monitoradded",
    "monitorremoved",
    "monitoraddedv2",
    "monitorremovedv2",
    "fullscreen",
    "pin",
    "configreloaded",
    "bringactivetotop",
];

/// Whether an event line (`name>>data`) is one the docking loop should react to.
fn is_window_event(line: &str) -> bool {
    let name = line.split_once(">>").map_or(line, |(name, _)| name);
    WINDOW_EVENTS.contains(&name)
}

/// Splits the bytes read so far into complete lines and leaves the unfinished one in `pending`.
/// Returns whether any complete line was a window event.
fn take_events(pending: &mut Vec<u8>, fresh: &[u8]) -> bool {
    pending.extend_from_slice(fresh);
    let mut hit = false;
    while let Some(end) = pending.iter().position(|b| *b == b'\n') {
        let line: Vec<u8> = pending.drain(..=end).collect();
        hit |= std::str::from_utf8(&line[..line.len() - 1]).is_ok_and(is_window_event);
    }
    if pending.len() > LINE_CAP {
        pending.clear();
    }
    hit
}

/// The best driver for the session: the sockets when this is Hyprland with its sockets in place,
/// else the `hyprctl` command, which is what ofolder used before.
pub fn compositor() -> Arc<dyn Compositor> {
    match HyprSocket::from_env() {
        Some(sock) => Arc::new(sock),
        None => Arc::new(Hyprctl),
    }
}

#[derive(Debug, Clone)]
pub struct HyprSocket {
    requests: PathBuf,
    events: PathBuf,
    /// Milliseconds since the epoch before which the request socket is not tried again.
    resting_until: Arc<AtomicU64>,
    fallback: Hyprctl,
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

impl HyprSocket {
    /// The sockets of the running Hyprland, or none when this is not a Hyprland session or the
    /// request socket is missing or not the user's own.
    pub fn from_env() -> Option<Self> {
        let runtime = PathBuf::from(std::env::var_os("XDG_RUNTIME_DIR")?);
        let signature = std::env::var("HYPRLAND_INSTANCE_SIGNATURE").ok()?;
        Self::in_dir(&runtime.join("hypr"), &signature)
    }

    fn in_dir(base: &Path, signature: &str) -> Option<Self> {
        // The signature becomes a path component, so it may not name another place.
        if signature.is_empty()
            || !signature
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        {
            return None;
        }
        let dir = base.join(signature);
        let requests = dir.join(".socket.sock");
        if !is_own_socket(&requests) {
            return None;
        }
        Some(Self {
            events: dir.join(".socket2.sock"),
            requests,
            resting_until: Arc::new(AtomicU64::new(0)),
            fallback: Hyprctl,
        })
    }

    /// Asks the request socket, or returns none when it is resting or fails (and then rests).
    fn ask(&self, command: &str) -> Option<String> {
        if now_ms() < self.resting_until.load(Ordering::Relaxed) {
            return None;
        }
        let reply =
            request_until_close(&self.requests, command.as_bytes(), REQUEST_LIMIT, REPLY_CAP)
                .ok()
                .and_then(|bytes| String::from_utf8(bytes).ok());
        if reply.is_none() {
            self.resting_until.store(
                now_ms() + REST_AFTER_FAILURE.as_millis() as u64,
                Ordering::Relaxed,
            );
        }
        reply
    }

    /// Reads through the socket and parses the reply; any failure of either is `None`, so the
    /// caller falls back to `hyprctl` and the error that gives is the one reported.
    fn read<T>(
        &self,
        command: &str,
        parse: impl FnOnce(&str) -> Result<T, SystemError>,
    ) -> Option<T> {
        parse(&self.ask(command)?).ok()
    }
}

/// Whether `path` is a socket owned by the same user as this process.
fn is_own_socket(path: &Path) -> bool {
    use std::os::unix::fs::{FileTypeExt, MetadataExt};
    let Ok(meta) = path.symlink_metadata() else {
        return false;
    };
    let me = std::fs::metadata("/proc/self").map(|m| m.uid()).ok();
    meta.file_type().is_socket() && me == Some(meta.uid())
}

impl Compositor for HyprSocket {
    fn clients(&self) -> Result<Vec<Client>, SystemError> {
        match self.read("j/clients", |t| {
            parse_clients(t).map_err(|e| SystemError(e.to_string()))
        }) {
            Some(clients) => Ok(clients),
            None => self.fallback.clients(),
        }
    }

    fn monitors(&self) -> Result<Vec<Monitor>, SystemError> {
        match self.read("j/monitors", |t| {
            parse_monitors(t).map_err(|e| SystemError(e.to_string()))
        }) {
            Some(monitors) => Ok(monitors),
            None => self.fallback.monitors(),
        }
    }

    fn active_window(&self) -> Result<Option<String>, SystemError> {
        match self.read("j/activewindow", parse_active) {
            Some(active) => Ok(active),
            None => self.fallback.active_window(),
        }
    }

    fn cursor(&self) -> Result<(i32, i32), SystemError> {
        match self.ask("j/cursorpos").as_deref().and_then(parse_cursor) {
            Some(at) => Ok(at),
            None => self.fallback.cursor(),
        }
    }

    fn focus(&self, address: &str) -> Result<(), SystemError> {
        self.fallback.focus(address)
    }

    fn float_window(
        &self,
        address: &str,
        size: (i32, i32),
        at: (i32, i32),
    ) -> Result<(), SystemError> {
        self.fallback.float_window(address, size, at)
    }

    fn resize_window(&self, address: &str, size: (i32, i32)) -> Result<(), SystemError> {
        self.fallback.resize_window(address, size)
    }

    fn move_window(&self, address: &str, at: (i32, i32)) -> Result<(), SystemError> {
        self.fallback.move_window(address, at)
    }

    fn send_to_workspace(&self, address: &str, workspace: &str) -> Result<(), SystemError> {
        self.fallback.send_to_workspace(address, workspace)
    }

    fn install_float_rule(
        &self,
        name: &str,
        class: &str,
        size: (i32, i32),
        at: (i32, i32),
        take_focus: bool,
    ) -> Result<(), SystemError> {
        self.fallback
            .install_float_rule(name, class, size, at, take_focus)
    }

    fn install_tip_rule(
        &self,
        name: &str,
        class: &str,
        size: (i32, i32),
        at: (i32, i32),
    ) -> Result<(), SystemError> {
        self.fallback.install_tip_rule(name, class, size, at)
    }

    fn remove_rule(&self, name: &str) -> Result<(), SystemError> {
        self.fallback.remove_rule(name)
    }

    fn binds(&self) -> Result<Vec<Bind>, SystemError> {
        self.fallback.binds()
    }

    fn bind_key(&self, combo: &Combo, command_line: &str, title: &str) -> Result<(), SystemError> {
        self.fallback.bind_key(combo, command_line, title)
    }

    fn unbind_key(&self, combo: &Combo) -> Result<(), SystemError> {
        self.fallback.unbind_key(combo)
    }

    fn watch_events(&self, wake: Arc<Wake>) -> Arc<AtomicBool> {
        let live = Arc::new(AtomicBool::new(false));
        let (path, flag) = (self.events.clone(), live.clone());
        let started = std::thread::Builder::new()
            .name("hypr-events".into())
            .spawn(move || follow_events(&path, &wake, &flag));
        if started.is_err() {
            live.store(false, Ordering::Relaxed);
        }
        live
    }
}

/// Reads the event socket for as long as the process runs, waking `wake` on each window event.
/// `live` is true only while connected: when the socket goes away it turns false, which sends the
/// docking loop back to its timer, and it turns true again (with a wake, in case something was
/// missed) when the socket is back.
fn follow_events(path: &Path, wake: &Wake, live: &AtomicBool) {
    loop {
        if let Ok(mut stream) = connect_within(path, REQUEST_LIMIT) {
            live.store(true, Ordering::Relaxed);
            wake.notify();
            let mut pending = Vec::new();
            let mut chunk = [0u8; 8192];
            loop {
                match stream.read(&mut chunk) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        if take_events(&mut pending, &chunk[..n]) {
                            wake.notify();
                        }
                    }
                }
            }
            live.store(false, Ordering::Relaxed);
            wake.notify();
        }
        // Not tried more than once in a while, so a missing socket costs nothing.
        let until = Instant::now() + RECONNECT;
        while Instant::now() < until {
            std::thread::sleep(Duration::from_millis(100));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::os::unix::net::UnixListener;

    fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("ofolder-hyprsock-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn only_window_events_are_taken_as_changes() {
        assert!(is_window_event("openwindow>>80a1,1,foo,Bar"));
        assert!(is_window_event("activewindowv2>>80a1"));
        assert!(is_window_event("closewindow>>80a1"));
        assert!(!is_window_event("submap>>reset"));
        assert!(!is_window_event("activelayout>>kbd,us"));
        assert!(!is_window_event(""));
        // A name that merely begins like one is not one.
        assert!(!is_window_event("openwindowx>>1"));
        // Text in the data never counts as an event name.
        assert!(!is_window_event("custom>>openwindow>>1"));
    }

    #[test]
    fn a_line_split_across_reads_is_put_together() {
        let mut pending = Vec::new();
        assert!(!take_events(&mut pending, b"submap>>reset\nclosew"));
        assert!(take_events(&mut pending, b"indow>>80a1\n"));
        assert!(pending.is_empty());
    }

    #[test]
    fn a_line_that_never_ends_is_dropped_not_kept_forever() {
        let mut pending = Vec::new();
        assert!(!take_events(&mut pending, &vec![b'a'; LINE_CAP + 1]));
        assert!(pending.is_empty());
    }

    #[test]
    fn a_signature_that_names_another_place_is_refused() {
        let base = scratch("sig");
        assert!(HyprSocket::in_dir(&base, "../x").is_none());
        assert!(HyprSocket::in_dir(&base, "").is_none());
        assert!(HyprSocket::in_dir(&base, "a/b").is_none());
        // A well-formed signature with no socket there is also refused.
        assert!(HyprSocket::in_dir(&base, "abc_123").is_none());
    }

    #[test]
    fn reads_go_to_the_socket_and_a_dead_socket_falls_back_to_the_command() {
        let base = scratch("reads");
        let dir = base.join("sig_1");
        std::fs::create_dir_all(&dir).unwrap();
        let listener = UnixListener::bind(dir.join(".socket.sock")).unwrap();
        let server = std::thread::spawn(move || {
            let (mut c, _) = listener.accept().unwrap();
            let mut ask = [0u8; 64];
            let n = c.read(&mut ask).unwrap();
            assert_eq!(&ask[..n], b"j/cursorpos");
            c.write_all(b"{\"x\": 12, \"y\": 34}").unwrap();
        });
        let sock = HyprSocket::in_dir(&base, "sig_1").expect("a socket of ours");
        assert_eq!(
            sock.ask("j/cursorpos").as_deref().and_then(parse_cursor),
            Some((12, 34))
        );
        server.join().unwrap();
        // The listener is gone: the next ask fails and the socket rests.
        assert!(sock.ask("j/clients").is_none());
        assert!(now_ms() < sock.resting_until.load(Ordering::Relaxed));
        assert!(sock.ask("j/clients").is_none());
    }

    #[test]
    fn events_wake_the_loop_and_the_flag_follows_the_connection() {
        let base = scratch("events");
        let dir = base.join("sig_2");
        std::fs::create_dir_all(&dir).unwrap();
        let request = UnixListener::bind(dir.join(".socket.sock")).unwrap();
        let events = UnixListener::bind(dir.join(".socket2.sock")).unwrap();
        let sock = HyprSocket::in_dir(&base, "sig_2").expect("a socket of ours");
        let wake = Arc::new(Wake::default());
        let live = sock.watch_events(wake.clone());
        let (mut peer, _) = events.accept().unwrap();
        // The connection itself wakes the loop once, so a change made meanwhile is seen.
        assert!(wake.wait(Duration::from_secs(2)));
        assert!(live.load(Ordering::Relaxed));
        peer.write_all(b"submap>>reset\n").unwrap();
        assert!(!wake.wait(Duration::from_millis(200)));
        peer.write_all(b"openwindow>>80a1,1,c,t\n").unwrap();
        assert!(wake.wait(Duration::from_secs(2)));
        drop(peer);
        assert!(wake.wait(Duration::from_secs(2)));
        assert!(!live.load(Ordering::Relaxed));
        drop(request);
    }
}
