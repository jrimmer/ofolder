//! Shortcuts at run time: binding the user's key combinations in Hyprland
//! while a dialog is open, and the socket `ofolder do <command>` uses to
//! reach the strip.
//!
//! Hyprland holds the bindings, not ofolder, so a strip that dies without
//! cleaning up would leave keys taken. The bindings are therefore written
//! down in a record under `$XDG_RUNTIME_DIR/ofolder/`, together with the
//! strip that made them, and whoever finds that strip dead removes them: the
//! next strip, the portal after it stops a strip, or `ofolder do`.

use std::collections::VecDeque;
use std::fs;
use std::io::{self, Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use ofolder_core::keybind::{do_command_line, taken_by};
use ofolder_core::shortcuts::{Combo, Shortcuts, command};
use ofolder_core::system::Compositor;

const RECORD: &str = "binds";
const SOCKET_PREFIX: &str = "strip-";
const SOCKET_SUFFIX: &str = ".sock";
/// A request is one command id and a newline; anything longer is refused.
const MAX_REQUEST: usize = 64;

/// Commands that arrived on the socket, for the strip to run.
pub type Queue = Arc<Mutex<VecDeque<&'static str>>>;

/// `$XDG_RUNTIME_DIR/ofolder`, made private to the user.
pub fn runtime_dir() -> io::Result<PathBuf> {
    let base = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .ok_or_else(|| io::Error::other("XDG_RUNTIME_DIR is not set"))?;
    let dir = base.join("ofolder");
    fs::create_dir_all(&dir)?;
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o700))?;
    Ok(dir)
}

/// Whether `pid` is a running ofolder process, so a reused pid is not taken
/// for a strip.
pub fn ofolder_alive(pid: u32) -> bool {
    fs::read_to_string(format!("/proc/{pid}/comm")).is_ok_and(|c| c.trim() == "ofolder")
}

// ---- the record --------------------------------------------------------

#[derive(Debug, PartialEq, Eq)]
pub struct Record {
    pub pid: u32,
    /// As `hl.bind` names them: `SUPER + ALT + F9`.
    pub keys: Vec<String>,
}

fn parse_record(text: &str) -> Option<Record> {
    let mut lines = text.lines();
    let pid = lines.next()?.trim().parse().ok()?;
    let keys = lines
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(str::to_owned)
        .collect();
    Some(Record { pid, keys })
}

pub fn read_record(dir: &Path) -> Option<Record> {
    parse_record(&fs::read_to_string(dir.join(RECORD)).ok()?)
}

fn write_record(dir: &Path, record: &Record) -> io::Result<()> {
    let mut text = format!("{}\n", record.pid);
    for key in &record.keys {
        text.push_str(key);
        text.push('\n');
    }
    let temp = dir.join(format!(".{RECORD}.{}.tmp", std::process::id()));
    fs::write(&temp, text)?;
    fs::rename(&temp, dir.join(RECORD))
}

/// Removes every binding ofolder made: those in the record, whoever made
/// them, and any binding described as ofolder's that the compositor still
/// holds. Returns how many were removed.
pub fn clear_ours(compositor: &dyn Compositor, dir: &Path) -> usize {
    let mut combos: Vec<Combo> = read_record(dir)
        .map(|r| r.keys)
        .unwrap_or_default()
        .iter()
        .filter_map(|k| Combo::parse(k).ok())
        .collect();
    if let Ok(binds) = compositor.binds() {
        combos.extend(
            binds
                .iter()
                .filter(|b| b.is_ours())
                .filter_map(|b| b.bind_keys())
                .filter_map(|k| Combo::parse(&k).ok()),
        );
    }
    combos.sort();
    combos.dedup();
    let removed = combos
        .iter()
        .filter(|c| compositor.unbind_key(c).is_ok())
        .count();
    fs::remove_file(dir.join(RECORD)).ok();
    removed
}

/// Removes the bindings and sockets of strips that are no longer running.
pub fn clean_stale(compositor: &dyn Compositor, dir: &Path, alive: &dyn Fn(u32) -> bool) {
    if let Some(record) = read_record(dir)
        && !alive(record.pid)
    {
        clear_ours(compositor, dir);
    }
    for (pid, path) in sockets(dir) {
        if !alive(pid) {
            fs::remove_file(path).ok();
        }
    }
}

/// After the portal stopped strip `pid`: its bindings go, if it left any.
pub fn release_for(compositor: &dyn Compositor, dir: &Path, pid: u32) {
    if read_record(dir).is_some_and(|r| r.pid == pid) {
        clear_ours(compositor, dir);
    }
}

// ---- binding -----------------------------------------------------------

/// The bindings of one strip.
pub struct Binder {
    compositor: Arc<dyn Compositor>,
    dir: PathBuf,
    exe: PathBuf,
    pid: u32,
}

/// A shortcut that could not be bound, and why, by command id.
pub type Problems = Vec<(String, String)>;

impl Binder {
    pub fn new(compositor: Arc<dyn Compositor>, dir: PathBuf, exe: PathBuf, pid: u32) -> Self {
        Binder {
            compositor,
            dir,
            exe,
            pid,
        }
    }

    /// Makes `shortcuts` the bound keys: whatever ofolder had bound goes
    /// first, then each combination the user's own configuration does not
    /// already bind is bound to this program's `do` command.
    pub fn apply(&self, shortcuts: &Shortcuts) -> Problems {
        clear_ours(&*self.compositor, &self.dir);
        let mut problems = Problems::new();
        if shortcuts.is_empty() {
            return problems;
        }
        let binds = match self.compositor.binds() {
            Ok(b) => b,
            Err(e) => {
                // Without the list, a combination could shadow one of the
                // user's own, so nothing is bound.
                for (def, _) in shortcuts.iter() {
                    problems.push((
                        def.id.to_owned(),
                        format!("not bound: the key bindings could not be read ({e})"),
                    ));
                }
                return problems;
            }
        };
        let mut bound = Vec::new();
        for (def, combo) in shortcuts.iter() {
            if let Some(holder) = taken_by(combo, &binds) {
                problems.push((
                    def.id.to_owned(),
                    format!("not bound: {} is already {holder}", combo.label()),
                ));
                continue;
            }
            let result = do_command_line(&self.exe, def.id)
                .map_err(|e| e.to_string())
                .and_then(|line| {
                    self.compositor
                        .bind_key(combo, &line, def.title)
                        .map_err(|e| e.to_string())
                });
            match result {
                Ok(()) => bound.push(combo.bind_keys()),
                Err(e) => problems.push((def.id.to_owned(), format!("not bound: {e}"))),
            }
        }
        if !bound.is_empty() {
            let record = Record {
                pid: self.pid,
                keys: bound,
            };
            if let Err(e) = write_record(&self.dir, &record) {
                // The bindings stay until the strip ends; only a crash would
                // leave them without a record.
                eprintln!("ofolder: could not record the key bindings: {e}");
            }
        }
        problems
    }

    /// Removes this strip's bindings, unless a newer strip has taken over.
    pub fn release(&self) {
        release_for(&*self.compositor, &self.dir, self.pid);
    }
}

// ---- the socket --------------------------------------------------------

fn socket_path(dir: &Path, pid: u32) -> PathBuf {
    dir.join(format!("{SOCKET_PREFIX}{pid}{SOCKET_SUFFIX}"))
}

/// Every strip socket in `dir`, by the pid in its name.
fn sockets(dir: &Path) -> Vec<(u32, PathBuf)> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().into_string().ok()?;
            let pid = name
                .strip_prefix(SOCKET_PREFIX)?
                .strip_suffix(SOCKET_SUFFIX)?
                .parse()
                .ok()?;
            Some((pid, e.path()))
        })
        .collect()
}

/// The command a request names, if it names one.
pub fn parse_request(bytes: &[u8]) -> Option<&'static str> {
    if bytes.len() > MAX_REQUEST {
        return None;
    }
    let text = std::str::from_utf8(bytes).ok()?.trim();
    command(text).map(|def| def.id)
}

/// Removes the socket when the strip ends.
pub struct Server {
    path: PathBuf,
}

impl Drop for Server {
    fn drop(&mut self) {
        fs::remove_file(&self.path).ok();
    }
}

/// Listens on this strip's socket and queues each known command.
pub fn serve(dir: &Path, pid: u32, queue: Queue) -> io::Result<Server> {
    let path = socket_path(dir, pid);
    fs::remove_file(&path).ok();
    let listener = UnixListener::bind(&path)?;
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            if let Some(id) = read_request(stream)
                && let Ok(mut q) = queue.lock()
            {
                q.push_back(id);
            }
        }
    });
    Ok(Server { path })
}

fn read_request(mut stream: UnixStream) -> Option<&'static str> {
    stream
        .set_read_timeout(Some(Duration::from_millis(200)))
        .ok()?;
    let mut buffer = Vec::new();
    Read::by_ref(&mut stream)
        .take(MAX_REQUEST as u64 + 1)
        .read_to_end(&mut buffer)
        .ok()?;
    parse_request(&buffer)
}

/// Sends `id` to the newest running strip. False when no strip answers; a
/// socket whose strip is gone is removed on the way.
pub fn send(dir: &Path, id: &str, alive: &dyn Fn(u32) -> bool) -> io::Result<bool> {
    let mut live: Vec<(std::time::SystemTime, PathBuf)> = Vec::new();
    for (pid, path) in sockets(dir) {
        if !alive(pid) {
            fs::remove_file(&path).ok();
            continue;
        }
        let made = fs::metadata(&path)
            .and_then(|m| m.modified())
            .unwrap_or(std::time::UNIX_EPOCH);
        live.push((made, path));
    }
    live.sort_by_key(|entry| std::cmp::Reverse(entry.0));
    for (_, path) in live {
        match UnixStream::connect(&path) {
            Ok(mut stream) => {
                stream.write_all(format!("{id}\n").as_bytes())?;
                return Ok(true);
            }
            Err(_) => {
                fs::remove_file(&path).ok();
            }
        }
    }
    Ok(false)
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex as StdMutex;

    use ofolder_core::hypr::{Client, Monitor};
    use ofolder_core::keybind::{Bind, OURS};
    use ofolder_core::system::SystemError;

    use super::*;

    /// A compositor that keeps bindings in a list.
    #[derive(Default)]
    struct Keys {
        binds: StdMutex<Vec<Bind>>,
        fail_list: bool,
    }

    impl Keys {
        fn with_user_bind(modmask: u32, key: &str, description: &str) -> Keys {
            let k = Keys::default();
            k.binds.lock().unwrap().push(Bind {
                modmask,
                key: key.to_owned(),
                description: description.to_owned(),
                submap: String::new(),
                mouse: false,
            });
            k
        }

        fn ours(&self) -> Vec<String> {
            let mut v: Vec<_> = self
                .binds
                .lock()
                .unwrap()
                .iter()
                .filter(|b| b.is_ours())
                .filter_map(|b| b.bind_keys())
                .collect();
            v.sort();
            v
        }
    }

    impl Compositor for Keys {
        fn clients(&self) -> Result<Vec<Client>, SystemError> {
            Ok(Vec::new())
        }
        fn monitors(&self) -> Result<Vec<Monitor>, SystemError> {
            Ok(Vec::new())
        }
        fn active_window(&self) -> Result<Option<String>, SystemError> {
            Ok(None)
        }
        fn focus(&self, _: &str) -> Result<(), SystemError> {
            Ok(())
        }
        fn binds(&self) -> Result<Vec<Bind>, SystemError> {
            if self.fail_list {
                return Err(SystemError("no hyprctl".into()));
            }
            Ok(self.binds.lock().unwrap().clone())
        }
        fn bind_key(&self, combo: &Combo, _: &str, title: &str) -> Result<(), SystemError> {
            self.binds.lock().unwrap().push(Bind {
                modmask: u32::from(combo.modmask()),
                key: combo.key().to_owned(),
                description: format!("{OURS}{title}"),
                submap: String::new(),
                mouse: false,
            });
            Ok(())
        }
        fn unbind_key(&self, combo: &Combo) -> Result<(), SystemError> {
            self.binds.lock().unwrap().retain(|b| {
                !(b.modmask == u32::from(combo.modmask())
                    && b.key.eq_ignore_ascii_case(combo.key()))
            });
            Ok(())
        }
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ofolder-keys-{}-{name}", std::process::id()));
        fs::remove_dir_all(&dir).ok();
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn shortcuts(pairs: &[(&str, &str)]) -> Shortcuts {
        let mut s = Shortcuts::default();
        for (id, keys) in pairs {
            assert!(s.set(id, Some(Combo::parse(keys).unwrap())));
        }
        s
    }

    fn binder(keys: &Arc<Keys>, dir: &Path, pid: u32) -> Binder {
        Binder::new(
            keys.clone(),
            dir.to_owned(),
            PathBuf::from("/usr/bin/ofolder"),
            pid,
        )
    }

    #[test]
    fn shortcuts_are_bound_recorded_and_released_with_the_strip() {
        let dir = scratch("bind");
        let keys = Arc::new(Keys::default());
        let b = binder(&keys, &dir, 41);
        let problems = b.apply(&shortcuts(&[("places", "SUPER+ALT+F9"), ("find", "F10")]));
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(keys.ours(), ["F10", "SUPER + ALT + F9"]);
        let record = read_record(&dir).expect("recorded");
        assert_eq!(record.pid, 41);
        assert_eq!(record.keys.len(), 2);
        b.release();
        assert!(keys.ours().is_empty());
        assert!(read_record(&dir).is_none());
    }

    #[test]
    fn a_combination_the_users_configuration_binds_is_skipped_and_reported() {
        let dir = scratch("taken");
        let keys = Arc::new(Keys::with_user_bind(72, "F", "Full width"));
        let problems = binder(&keys, &dir, 7).apply(&shortcuts(&[
            ("places", "SUPER+ALT+F"),
            ("find", "SUPER+ALT+G"),
        ]));
        assert_eq!(problems.len(), 1);
        assert_eq!(problems[0].0, "places");
        assert!(problems[0].1.contains("Full width"), "{}", problems[0].1);
        assert_eq!(keys.ours(), ["SUPER + ALT + G"]);
        // The user's own binding is still there.
        assert_eq!(keys.binds.lock().unwrap().len(), 2);
    }

    #[test]
    fn nothing_is_bound_when_the_bindings_cannot_be_read() {
        let dir = scratch("unreadable");
        let keys = Arc::new(Keys {
            fail_list: true,
            ..Keys::default()
        });
        let problems = binder(&keys, &dir, 7).apply(&shortcuts(&[("places", "F9")]));
        assert_eq!(problems.len(), 1);
        assert!(keys.binds.lock().unwrap().is_empty());
    }

    #[test]
    fn a_change_replaces_the_old_bindings() {
        let dir = scratch("change");
        let keys = Arc::new(Keys::default());
        let b = binder(&keys, &dir, 7);
        b.apply(&shortcuts(&[("places", "F9")]));
        b.apply(&shortcuts(&[("places", "F11")]));
        assert_eq!(keys.ours(), ["F11"]);
        b.apply(&Shortcuts::default());
        assert!(keys.ours().is_empty());
        assert!(read_record(&dir).is_none());
    }

    #[test]
    fn a_newer_strip_takes_the_keys_and_the_older_one_leaves_them_alone() {
        let dir = scratch("newer");
        let keys = Arc::new(Keys::default());
        let old = binder(&keys, &dir, 1);
        old.apply(&shortcuts(&[("places", "F9")]));
        let new = binder(&keys, &dir, 2);
        new.apply(&shortcuts(&[("places", "F9")]));
        assert_eq!(keys.ours(), ["F9"], "bound once, not twice");
        old.release();
        assert_eq!(
            keys.ours(),
            ["F9"],
            "the older strip must not unbind the newer one's"
        );
        new.release();
        assert!(keys.ours().is_empty());
    }

    #[test]
    fn a_dead_strips_bindings_and_socket_are_cleaned_up() {
        let dir = scratch("stale");
        let keys = Arc::new(Keys::default());
        binder(&keys, &dir, 99).apply(&shortcuts(&[("terminal", "SUPER+ALT+T")]));
        fs::write(socket_path(&dir, 99), "").unwrap();
        fs::write(socket_path(&dir, 5), "").unwrap();
        clean_stale(&*keys, &dir, &|pid| pid == 5);
        assert!(keys.ours().is_empty());
        assert!(!socket_path(&dir, 99).exists());
        assert!(socket_path(&dir, 5).exists(), "a live strip's socket stays");
        // A live owner keeps its bindings.
        binder(&keys, &dir, 5).apply(&shortcuts(&[("terminal", "SUPER+ALT+T")]));
        clean_stale(&*keys, &dir, &|pid| pid == 5);
        assert_eq!(keys.ours(), ["SUPER + ALT + T"]);
    }

    #[test]
    fn bindings_described_as_ofolders_are_cleared_even_without_a_record() {
        let dir = scratch("orphan");
        let keys = Arc::new(Keys::with_user_bind(64, "RETURN", "Terminal"));
        keys.bind_key(&Combo::parse("F9").unwrap(), "x", "Places")
            .unwrap();
        assert_eq!(clear_ours(&*keys, &dir), 1);
        assert!(keys.ours().is_empty());
        assert_eq!(
            keys.binds.lock().unwrap().len(),
            1,
            "the user's binding stays"
        );
    }

    #[test]
    fn the_portal_releases_only_the_bindings_of_the_strip_it_stopped() {
        let dir = scratch("release-for");
        let keys = Arc::new(Keys::default());
        binder(&keys, &dir, 8).apply(&shortcuts(&[("places", "F9")]));
        release_for(&*keys, &dir, 3);
        assert_eq!(keys.ours(), ["F9"]);
        release_for(&*keys, &dir, 8);
        assert!(keys.ours().is_empty());
    }

    #[test]
    fn a_record_is_a_pid_then_one_combination_per_line() {
        assert_eq!(
            parse_record("12\nSUPER + ALT + F9\n\nF10\n"),
            Some(Record {
                pid: 12,
                keys: vec!["SUPER + ALT + F9".into(), "F10".into()]
            })
        );
        assert_eq!(parse_record("x\nF9\n"), None);
        assert_eq!(parse_record(""), None);
    }

    #[test]
    fn only_a_known_command_is_taken_from_a_request() {
        assert_eq!(parse_request(b"places\n"), Some("places"));
        assert_eq!(parse_request(b"  find  "), Some("find"));
        assert_eq!(parse_request(b"rm -rf /\n"), None);
        assert_eq!(parse_request(&[b'a'; 200]), None);
        assert_eq!(parse_request(&[0xff, 0xfe]), None);
    }

    #[test]
    fn a_command_reaches_the_newest_running_strip_over_its_socket() {
        let dir = scratch("socket");
        let queue: Queue = Arc::default();
        let server = serve(&dir, 4242, queue.clone()).expect("listens");
        let mode = fs::metadata(socket_path(&dir, 4242))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
        // A socket file nobody listens on is stale and removed.
        fs::write(socket_path(&dir, 77), "").unwrap();
        assert!(send(&dir, "places", &|pid| pid == 4242 || pid == 77).unwrap());
        let mut got = None;
        for _ in 0..100 {
            if let Some(id) = queue.lock().unwrap().pop_front() {
                got = Some(id);
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(got, Some("places"));
        drop(server);
        assert!(!socket_path(&dir, 4242).exists());
        assert!(!send(&dir, "places", &|_| true).unwrap());
    }
}
