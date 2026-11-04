//! The item selected in a dialog's file list, for the preview panel.
//!
//! Each dialog says what is selected in its own way, and none of them says
//! it as a path: Flea gives the cursor row's name beside the folder, GTK and
//! Strata give a row's label. A name is only turned into a path when it is
//! one plain file name inside the folder the dialog shows and that path
//! exists; anything else is `None`.

use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use ofolder_core::exec::run_with_deadline;
use serde_json::Value;

use crate::a11y::{Node, Role, walk};
use crate::sockio::request_line;

/// `name` inside `folder`, when `name` is a single plain file name and the
/// result exists (a broken link counts: it is still the item selected).
pub fn name_in_folder(folder: &Path, name: &str) -> Option<PathBuf> {
    let mut parts = Path::new(name).components();
    let (Some(Component::Normal(_)), None) = (parts.next(), parts.next()) else {
        return None;
    };
    if !folder.is_absolute() {
        return None;
    }
    let path = folder.join(name);
    path.symlink_metadata().ok().map(|_| path)
}

/// What Flea's picker answers to its `snapshot` call: the folder, the
/// cursor row's name and the marked rows. A single mark wins over the cursor;
/// several marks are no single selection. Not checked against the disk.
pub fn flea_candidate(snapshot: &str) -> Option<PathBuf> {
    let value: Value = serde_json::from_str(snapshot).ok()?;
    let marks = value.get("marks")?.as_array()?;
    match marks.as_slice() {
        [] => {}
        [mark] => {
            let path = PathBuf::from(mark.get("path")?.as_str()?);
            return path.is_absolute().then_some(path);
        }
        _ => return None,
    }
    let folder = Path::new(value.get("path")?.as_str()?);
    let name = value.get("cursorName")?.as_str()?;
    // The Recent view's "folder" is not a folder, and its rows are not in one.
    if !folder.is_absolute() || name.is_empty() {
        return None;
    }
    let mut parts = Path::new(name).components();
    let (Some(Component::Normal(_)), None) = (parts.next(), parts.next()) else {
        return None;
    };
    Some(folder.join(name))
}

/// The folder Flea's picker reports in its `snapshot`, when it is an
/// absolute path to a folder that exists. The Recent view reports no folder.
pub fn flea_snapshot_folder(snapshot: &str) -> Option<PathBuf> {
    let value: Value = serde_json::from_str(snapshot).ok()?;
    let folder = PathBuf::from(value.get("path")?.as_str()?);
    (folder.is_absolute() && folder.is_dir()).then_some(folder)
}

/// Whether the picker's path bar is open, from its `snapshot`. `None` when
/// the snapshot does not say, which is never read as either answer.
pub fn flea_path_bar_open(snapshot: &str) -> Option<bool> {
    serde_json::from_str::<Value>(snapshot)
        .ok()?
        .get("pathBarOpen")?
        .as_bool()
}

/// Asks a running Flea picker for its `snapshot`. The picker answers this
/// read-only call on its own IPC; nothing about the dialog changes.
pub trait PickerIpc: Send + Sync {
    fn snapshot(&self, pid: u32) -> Option<String>;

    /// Calls a read-only function of a Flea window's IPC (`flea.path` for a
    /// file-manager window) and returns the one line it answered.
    fn call(&self, _pid: u32, _target: &str, _function: &str) -> Option<String> {
        None
    }
}

/// The answer of a call: its last non-empty line, since `qs` may log a
/// warning before it.
fn last_line(output: &str) -> Option<String> {
    output
        .lines()
        .map(str::trim)
        .rfind(|l| !l.is_empty())
        .map(str::to_owned)
}

/// Longer than a call has ever taken (about 18 ms), short enough that a
/// stuck picker cannot hold the poll up.
const IPC_LIMIT: Duration = Duration::from_millis(800);

/// Runs `command` and returns its output when it succeeds within
/// [`IPC_LIMIT`]; a stuck client is killed.
fn run_limited(mut command: Command) -> Option<String> {
    let output = run_with_deadline(&mut command, IPC_LIMIT).ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8(output.stdout).ok()
}

/// The picker's IPC through `qs ipc`, Quickshell's own client. This is how
/// Flea 0.3.x's picker, a Quickshell window, is reached.
#[derive(Debug, Default, Clone, Copy)]
pub struct QsIpc;

impl PickerIpc for QsIpc {
    fn snapshot(&self, pid: u32) -> Option<String> {
        let mut command = Command::new("qs");
        command.args([
            "ipc",
            "--pid",
            &pid.to_string(),
            "call",
            "fleapicker",
            "snapshot",
        ]);
        run_limited(command).map(|out| json_from_first_object(&out))
    }

    fn call(&self, pid: u32, target: &str, function: &str) -> Option<String> {
        let mut command = Command::new("qs");
        command.args(["ipc", "--pid", &pid.to_string(), "call", target, function]);
        run_limited(command).and_then(|out| last_line(&out))
    }
}

/// `qs` logs a warning on its output before the answer when the process has
/// no usable locale (a service started by D-Bus often has none), so the
/// answer starts at the first line that opens a JSON object.
fn json_from_first_object(output: &str) -> String {
    let mut offset = 0;
    for line in output.split_inclusive('\n') {
        if line.trim_start().starts_with('{') {
            return output[offset..].to_owned();
        }
        offset += line.len();
    }
    output.to_owned()
}

/// The picker's IPC. Flea 0.4's picker is a native window that serves a call on its own socket,
/// one JSON line in (`{"call":[target,function,...]}`) and one out
/// (`{"code":0,"stdout":"...","stderr":""}`); this asks it there, which costs no new process.
/// `flea ipc`, Flea's own client for the same socket, is the fallback for anything that is not a
/// clean answer on the socket: a missing or untrusted socket, a silent picker, a reply that is
/// not the expected shape. A picker that answers with a non-zero code has refused, as `flea ipc`
/// would report, and that is not retried.
#[derive(Debug, Clone)]
pub struct FleaIpc {
    program: PathBuf,
    sockets: Option<PathBuf>,
    /// Milliseconds since the epoch before which the socket is not tried again after a failure.
    resting_until: Arc<AtomicU64>,
}

/// How long the picker has to answer on its socket. A call has never taken more than about 18 ms.
const SOCKET_LIMIT: Duration = Duration::from_millis(400);
/// After a failure on the socket, the command is used for this long before the socket is tried.
const SOCKET_REST: Duration = Duration::from_secs(5);
/// The largest reply taken from a picker (a snapshot of a long folder is the biggest).
const REPLY_CAP: usize = 4 * 1024 * 1024;

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

/// What a picker said on its socket.
#[derive(Debug, PartialEq, Eq)]
enum Reply {
    Answer(String),
    Refused,
}

/// The reply line of a picker, or none when it is not the documented shape.
fn parse_reply(line: &[u8]) -> Option<Reply> {
    let value: serde_json::Value = serde_json::from_slice(line).ok()?;
    let code = value.get("code")?.as_i64()?;
    let stdout = value.get("stdout")?.as_str()?;
    Some(if code == 0 {
        Reply::Answer(stdout.to_owned())
    } else {
        Reply::Refused
    })
}

/// Whether `dir` and `socket` belong to this user and are closed to everyone else, as `flea ipc`
/// itself insists before it connects.
fn trusted(dir: &Path, socket: &Path) -> bool {
    use std::os::unix::fs::{FileTypeExt, MetadataExt};
    let Ok(me) = std::fs::metadata("/proc/self").map(|m| m.uid()) else {
        return false;
    };
    let Ok(dir_meta) = std::fs::symlink_metadata(dir) else {
        return false;
    };
    let Ok(sock_meta) = std::fs::symlink_metadata(socket) else {
        return false;
    };
    dir_meta.is_dir()
        && dir_meta.uid() == me
        && dir_meta.mode() & 0o077 == 0
        && sock_meta.file_type().is_socket()
        && sock_meta.uid() == me
        && sock_meta.mode() & 0o077 == 0
}

impl FleaIpc {
    /// A client that only runs `program ipc`, for tests and for a picker with no socket to try.
    pub fn new(program: impl Into<PathBuf>) -> Self {
        Self {
            program: program.into(),
            sockets: None,
            resting_until: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Asks pickers on their sockets in `dir` first.
    #[must_use]
    pub fn with_sockets(mut self, dir: impl Into<PathBuf>) -> Self {
        self.sockets = Some(dir.into());
        self
    }

    /// The call on the picker's socket: `Some` when the picker answered or refused, `None` when
    /// the socket could not settle it and the command should be asked.
    fn on_socket(&self, pid: u32, call: &[&str]) -> Option<Option<String>> {
        let dir = self.sockets.as_ref()?;
        if now_ms() < self.resting_until.load(Ordering::Relaxed) {
            return None;
        }
        let socket = dir.join(format!("picker-{pid}.sock"));
        if !trusted(dir, &socket) {
            return None;
        }
        let mut request = serde_json::json!({ "call": call }).to_string();
        request.push('\n');
        let reply = request_line(&socket, request.as_bytes(), SOCKET_LIMIT, REPLY_CAP)
            .ok()
            .and_then(|line| parse_reply(&line));
        match reply {
            Some(Reply::Answer(out)) => Some(Some(out)),
            Some(Reply::Refused) => Some(None),
            None => {
                self.resting_until
                    .store(now_ms() + SOCKET_REST.as_millis() as u64, Ordering::Relaxed);
                None
            }
        }
    }

    fn by_command(&self, pid: u32, call: &[&str]) -> Option<String> {
        let mut command = Command::new(&self.program);
        command.args(["ipc", "--pid", &pid.to_string(), "call"]);
        command.args(call);
        run_limited(command)
    }
}

impl Default for FleaIpc {
    fn default() -> Self {
        let runtime =
            std::env::var_os("XDG_RUNTIME_DIR").map_or_else(std::env::temp_dir, PathBuf::from);
        Self::new("flea").with_sockets(runtime.join("flea-ipc"))
    }
}

impl PickerIpc for FleaIpc {
    fn snapshot(&self, pid: u32) -> Option<String> {
        let call = ["fleapicker", "snapshot"];
        self.on_socket(pid, &call)
            .unwrap_or_else(|| self.by_command(pid, &call))
    }

    fn call(&self, pid: u32, target: &str, function: &str) -> Option<String> {
        let call = [target, function];
        self.on_socket(pid, &call)
            .unwrap_or_else(|| self.by_command(pid, &call))
            .and_then(|out| last_line(&out))
    }
}

/// Reaches whichever picker is running. A 0.4 picker binds
/// `$XDG_RUNTIME_DIR/flea-ipc/picker-<pid>.sock`; a 0.3 picker does not, so
/// the socket for this very pid decides, and no version is ever guessed. A
/// dead picker leaves its socket file behind, which is harmless because the
/// caller only asks for a pid it has found alive.
pub struct AutoIpc {
    sockets: PathBuf,
    native: Box<dyn PickerIpc>,
    quickshell: Box<dyn PickerIpc>,
}

impl AutoIpc {
    pub fn new(sockets: impl Into<PathBuf>) -> Self {
        Self {
            sockets: sockets.into(),
            native: Box::new(FleaIpc::default()),
            quickshell: Box::new(QsIpc),
        }
    }

    /// The two clients replaced, for tests.
    #[must_use]
    pub fn with_clients(
        mut self,
        native: Box<dyn PickerIpc>,
        quickshell: Box<dyn PickerIpc>,
    ) -> Self {
        self.native = native;
        self.quickshell = quickshell;
        self
    }

    fn has_native_socket(&self, pid: u32) -> bool {
        use std::os::unix::fs::FileTypeExt;
        self.sockets
            .join(format!("picker-{pid}.sock"))
            .symlink_metadata()
            .is_ok_and(|m| m.file_type().is_socket())
    }
}

impl Default for AutoIpc {
    fn default() -> Self {
        let runtime =
            std::env::var_os("XDG_RUNTIME_DIR").map_or_else(std::env::temp_dir, PathBuf::from);
        Self::new(runtime.join("flea-ipc"))
    }
}

impl PickerIpc for AutoIpc {
    fn snapshot(&self, pid: u32) -> Option<String> {
        if self.has_native_socket(pid) {
            self.native.snapshot(pid)
        } else {
            self.quickshell.snapshot(pid)
        }
    }

    /// A window of either generation: Flea 0.4's client answers for its own
    /// windows and refuses (or finds no socket) for a Quickshell one, and the
    /// other way round, so each is asked in turn.
    fn call(&self, pid: u32, target: &str, function: &str) -> Option<String> {
        self.native
            .call(pid, target, function)
            .or_else(|| self.quickshell.call(pid, target, function))
    }
}

/// The one selected row's name in GTK's file table ("Files"): its rows are
/// runs of cells, one per column header, and the first cell of a row holds
/// the icon and then the name.
pub fn gtk_selected_name<N: Node>(chooser: &N) -> Option<String> {
    let mut table = None;
    walk(chooser, &mut |node| {
        if node.role() == Role::Table && node.name() == "Files" {
            table = Some(node.clone());
            return crate::a11y::Walk::Stop;
        }
        if node.role() == Role::List {
            crate::a11y::Walk::Over
        } else {
            crate::a11y::Walk::Into
        }
    });
    let children = table?.children();
    let columns = children
        .iter()
        .filter(|c| c.role() == Role::ColumnHeader)
        .count();
    if columns == 0 {
        return None;
    }
    let cells: Vec<&N> = children
        .iter()
        .filter(|c| c.role() == Role::TableCell)
        .collect();
    let mut names = cells
        .chunks(columns)
        .filter_map(|row| row.first())
        .filter(|first| first.selected())
        .map(|first| {
            first
                .children()
                .iter()
                .map(Node::name)
                .rfind(|n| !n.is_empty())
                .unwrap_or_default()
        });
    match (names.next(), names.next()) {
        (Some(name), None) if !name.is_empty() => Some(name),
        _ => None,
    }
}

/// The one selected item in Strata's column for `folder`: each column is a
/// list named after its folder, holding one item per file. Two columns with
/// that name (a folder inside one of the same name) are ambiguous.
pub fn strata_selected_name<N: Node>(root: &N, folder: &Path) -> Option<String> {
    let wanted = folder.file_name()?.to_str()?.to_owned();
    let mut lists = Vec::new();
    walk(root, &mut |node| {
        if node.role() == Role::List {
            if node.name() == wanted {
                lists.push(node.clone());
            }
            return crate::a11y::Walk::Over;
        }
        crate::a11y::Walk::Into
    });
    let [list] = lists.as_slice() else {
        return None;
    };
    let mut names = list
        .children()
        .into_iter()
        .filter(|item| item.role() == Role::ListItem && item.selected())
        .map(|item| item.name());
    match (names.next(), names.next()) {
        (Some(name), None) if !name.is_empty() => Some(name),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::a11y::fake::FakeNode;

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ofolder-sel-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Answers with its own name, so a test can see which client was asked.
    struct Named(&'static str);

    impl PickerIpc for Named {
        fn snapshot(&self, _pid: u32) -> Option<String> {
            Some(self.0.to_owned())
        }
    }

    fn auto(dir: &Path) -> AutoIpc {
        AutoIpc::new(dir).with_clients(Box::new(Named("native")), Box::new(Named("qs")))
    }

    #[test]
    fn the_path_bar_state_is_read_only_when_the_snapshot_states_it() {
        assert_eq!(flea_path_bar_open(r#"{"pathBarOpen":true}"#), Some(true));
        assert_eq!(flea_path_bar_open(r#"{"pathBarOpen":false}"#), Some(false));
        for unsaid in [
            r#"{"path":"/x"}"#,
            r#"{"pathBarOpen":null}"#,
            r#"{"pathBarOpen":"yes"}"#,
            "nope",
            "",
        ] {
            assert_eq!(flea_path_bar_open(unsaid), None, "{unsaid}");
        }
    }

    #[test]
    fn a_warning_before_the_answer_does_not_hide_the_answer() {
        let warned = "\u{1b}[33m  WARN\u{1b}[0m: Detected locale \"C\", which is not UTF-8.\n{\"path\":\"/a\",\"pathBarOpen\":true}\n";
        let snapshot = json_from_first_object(warned);
        assert_eq!(snapshot, "{\"path\":\"/a\",\"pathBarOpen\":true}\n");
        assert_eq!(flea_path_bar_open(&snapshot), Some(true));
        assert_eq!(json_from_first_object("{\"a\":1}"), "{\"a\":1}");
        assert_eq!(
            json_from_first_object("no object here\n"),
            "no object here\n"
        );
    }

    #[test]
    fn a_picker_with_a_native_socket_is_asked_through_flea() {
        let dir = scratch("ipc-native");
        let _socket = std::os::unix::net::UnixListener::bind(dir.join("picker-42.sock")).unwrap();
        assert_eq!(auto(&dir).snapshot(42).as_deref(), Some("native"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_picker_without_a_native_socket_is_asked_through_quickshell() {
        let dir = scratch("ipc-qs");
        assert_eq!(auto(&dir).snapshot(42).as_deref(), Some("qs"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn only_the_socket_of_that_very_pid_decides_the_transport() {
        let dir = scratch("ipc-pid");
        let _socket = std::os::unix::net::UnixListener::bind(dir.join("picker-41.sock")).unwrap();
        assert_eq!(auto(&dir).snapshot(42).as_deref(), Some("qs"));
        assert_eq!(auto(&dir).snapshot(41).as_deref(), Some("native"));
        std::fs::remove_dir_all(&dir).ok();
    }

    /// A private folder with a picker's socket in it, served by `reply` for each request line.
    fn picker_socket(
        tag: &str,
        pid: u32,
        reply: impl Fn(&str) -> Vec<u8> + Send + 'static,
    ) -> PathBuf {
        use std::io::{BufRead, BufReader, Write};
        use std::os::unix::fs::PermissionsExt;
        let dir = scratch(tag);
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        let socket = dir.join(format!("picker-{pid}.sock"));
        let listener = std::os::unix::net::UnixListener::bind(&socket).unwrap();
        std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600)).unwrap();
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let mut line = String::new();
                BufReader::new(&stream).read_line(&mut line).ok();
                let mut stream = stream;
                stream.write_all(&reply(&line)).ok();
            }
        });
        dir
    }

    fn socket_only(dir: &Path) -> FleaIpc {
        FleaIpc::new("/nonexistent/flea-for-ofolder-test").with_sockets(dir)
    }

    #[test]
    fn a_picker_is_asked_on_its_own_socket_with_the_documented_line() {
        let dir = picker_socket("ipc-line", 7, |line| {
            assert_eq!(line, "{\"call\":[\"fleapicker\",\"snapshot\"]}\n");
            b"{\"code\":0,\"stdout\":\"{\\\"path\\\":\\\"/a b\\\"}\\n\",\"stderr\":\"\"}\n".to_vec()
        });
        let ipc = socket_only(&dir);
        assert_eq!(ipc.snapshot(7).as_deref(), Some("{\"path\":\"/a b\"}\n"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_call_returns_the_last_line_of_the_picker_output() {
        let dir = picker_socket("ipc-call", 8, |line| {
            assert_eq!(line, "{\"call\":[\"flea\",\"path\"]}\n");
            b"{\"code\":0,\"stdout\":\"/tmp/x\\n\",\"stderr\":\"\"}\n".to_vec()
        });
        assert_eq!(
            socket_only(&dir).call(8, "flea", "path").as_deref(),
            Some("/tmp/x")
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_refusal_on_the_socket_is_an_answer_of_none_not_a_retry() {
        let dir = picker_socket("ipc-refuse", 9, |_| {
            b"{\"code\":1,\"stdout\":\"\",\"stderr\":\"no\"}\n".to_vec()
        });
        let ipc = socket_only(&dir);
        assert_eq!(ipc.snapshot(9), None);
        // Not a failure of the socket, so it is not put to rest.
        assert_eq!(ipc.resting_until.load(Ordering::Relaxed), 0);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_reply_that_is_not_the_documented_shape_sends_the_call_to_the_command() {
        for (tag, junk) in [
            ("ipc-junk", b"nonsense\n".to_vec()),
            ("ipc-nocode", b"{\"stdout\":\"x\"}\n".to_vec()),
            ("ipc-quiet", Vec::new()),
        ] {
            let dir = picker_socket(tag, 10, move |_| junk.clone());
            let ipc = socket_only(&dir);
            // The command is missing in this test, so the fallback answers none.
            assert_eq!(ipc.snapshot(10), None, "{tag}");
            assert!(ipc.resting_until.load(Ordering::Relaxed) > 0, "{tag}");
            std::fs::remove_dir_all(&dir).ok();
        }
    }

    #[test]
    fn a_missing_socket_goes_to_the_command_without_resting() {
        let dir = scratch("ipc-nosock");
        let ipc = socket_only(&dir);
        assert_eq!(ipc.snapshot(11), None);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_socket_in_a_folder_others_can_enter_is_not_trusted() {
        use std::os::unix::fs::PermissionsExt;
        let dir = picker_socket("ipc-open", 12, |_| {
            b"{\"code\":0,\"stdout\":\"x\",\"stderr\":\"\"}\n".to_vec()
        });
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(socket_only(&dir).snapshot(12), None);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_silent_picker_is_given_up_on_at_the_deadline() {
        let dir = picker_socket("ipc-silent", 13, |_| {
            std::thread::sleep(Duration::from_secs(3));
            Vec::new()
        });
        let started = std::time::Instant::now();
        assert_eq!(socket_only(&dir).snapshot(13), None);
        assert!(started.elapsed() < Duration::from_secs(2));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_plain_file_named_like_a_socket_does_not_count() {
        let dir = scratch("ipc-file");
        std::fs::write(dir.join("picker-42.sock"), "").unwrap();
        assert_eq!(auto(&dir).snapshot(42).as_deref(), Some("qs"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_missing_socket_folder_means_quickshell() {
        let missing = std::env::temp_dir().join("ofolder-sel-no-such-dir");
        assert_eq!(auto(&missing).snapshot(42).as_deref(), Some("qs"));
    }

    #[test]
    fn flea_ipc_gives_nothing_when_the_client_is_missing() {
        let ipc = FleaIpc::new("/nonexistent/flea-for-ofolder-test");
        assert_eq!(ipc.snapshot(42), None);
    }

    #[test]
    fn a_client_that_fails_gives_nothing_and_one_that_hangs_is_cut_off() {
        let mut fails = Command::new("sh");
        fails.args(["-c", "echo partial; exit 3"]);
        assert_eq!(run_limited(fails), None);
        let mut hangs = Command::new("sh");
        hangs.args(["-c", "sleep 5"]);
        let started = std::time::Instant::now();
        assert_eq!(run_limited(hangs), None);
        assert!(started.elapsed() < Duration::from_secs(3));
        let mut works = Command::new("sh");
        works.args(["-c", "printf '{\"path\":\"/x\"}'"]);
        assert_eq!(run_limited(works).as_deref(), Some("{\"path\":\"/x\"}"));
    }

    #[test]
    fn a_plain_name_that_exists_in_the_folder_becomes_its_path() {
        let dir = scratch("plain");
        std::fs::write(dir.join("alpha.txt"), "x").unwrap();
        assert_eq!(
            name_in_folder(&dir, "alpha.txt"),
            Some(dir.join("alpha.txt"))
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_name_that_is_not_there_or_not_a_plain_name_is_none() {
        let dir = scratch("odd");
        std::fs::write(dir.join("a"), "x").unwrap();
        assert_eq!(name_in_folder(&dir, "missing.txt"), None);
        for odd in ["", ".", "..", "a/b", "/etc/passwd", "../a"] {
            assert_eq!(name_in_folder(&dir, odd), None, "{odd:?}");
        }
        assert_eq!(name_in_folder(Path::new("relative"), "a"), None);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn flea_reads_the_cursor_row_inside_the_folder_when_nothing_is_marked() {
        let snap = r#"{"path":"/tmp/pick","cursor":1,"cursorName":"aether.png","marks":[]}"#;
        assert_eq!(
            flea_candidate(snap),
            Some(PathBuf::from("/tmp/pick/aether.png"))
        );
    }

    #[test]
    fn flea_prefers_a_single_mark_and_refuses_several() {
        let one = r#"{"path":"/tmp/pick","cursorName":"a","marks":[{"path":"/tmp/pick/b","uri":"file:///tmp/pick/b"}]}"#;
        assert_eq!(flea_candidate(one), Some(PathBuf::from("/tmp/pick/b")));
        let two = r#"{"path":"/tmp/pick","cursorName":"a","marks":[{"path":"/tmp/pick/b"},{"path":"/tmp/pick/c"}]}"#;
        assert_eq!(flea_candidate(two), None);
    }

    #[test]
    fn flea_gives_nothing_for_the_recent_view_an_empty_row_or_a_bad_reply() {
        for snap in [
            r#"{"path":"recent:///","cursorName":"home/me/a.txt","marks":[]}"#,
            r#"{"path":"/tmp/pick","cursorName":"","marks":[]}"#,
            r#"{"path":"/tmp/pick","cursorName":"../x","marks":[]}"#,
            r#"{"path":"/tmp/pick","cursorName":"a"}"#,
            "not json",
            "",
        ] {
            assert_eq!(flea_candidate(snap), None, "{snap}");
        }
    }

    /// The shape dumped from the real GTK dialog: four column headers, then
    /// four cells per row; the first cell holds the icon and the name.
    fn gtk_table(selected: &[usize]) -> FakeNode {
        let chooser = FakeNode::root(Role::FileChooser, "Open");
        let places = chooser.add(Role::List, "");
        places.add(Role::TableCell, "not-a-file").selected_on();
        let table = chooser.add(Role::Table, "Files");
        for h in ["Name", "Size", "Type", "Modified"] {
            table.add(Role::ColumnHeader, h);
        }
        for (i, name) in ["gamma.md", "pinentry.png", "omarchy.png"]
            .iter()
            .enumerate()
        {
            let pick = |n: FakeNode| {
                if selected.contains(&i) {
                    n.selected_on()
                } else {
                    n
                }
            };
            let first = pick(table.add(Role::TableCell, ""));
            first.add(Role::TableCell, "");
            first.add(Role::TableCell, name);
            pick(table.add(Role::TableCell, "1 byte"));
            pick(table.add(Role::TableCell, "Document"));
            let modified = pick(table.add(Role::TableCell, ""));
            modified.add(Role::TableCell, "00:29");
            modified.add(Role::TableCell, "00:29");
        }
        chooser
    }

    #[test]
    fn gtk_reads_the_name_cell_of_the_one_selected_row() {
        assert_eq!(
            gtk_selected_name(&gtk_table(&[1])),
            Some("pinentry.png".to_owned())
        );
    }

    #[test]
    fn gtk_gives_nothing_when_no_row_or_several_rows_are_selected() {
        assert_eq!(gtk_selected_name(&gtk_table(&[])), None);
        assert_eq!(gtk_selected_name(&gtk_table(&[0, 2])), None);
    }

    fn strata_columns(folder_lists: &[&str], selected: &[&str]) -> FakeNode {
        let root = FakeNode::root(Role::Other, "pick — Strata");
        for list_name in folder_lists {
            let list = root.add(Role::Other, "column").add(Role::List, list_name);
            for item in ["sub-folder", "aether.png", "alpha.txt"] {
                let node = list.add(Role::ListItem, item);
                node.add(Role::Label, item);
                if selected.contains(&item) {
                    node.selected_on();
                }
            }
        }
        root
    }

    #[test]
    fn strata_reads_the_selected_item_of_the_column_for_the_folder_shown() {
        // The parent's column has its own selection; only the folder's column counts.
        let root = strata_columns(&["tmp", "pick"], &["alpha.txt"]);
        assert_eq!(
            strata_selected_name(&root, Path::new("/tmp/pick")),
            Some("alpha.txt".to_owned())
        );
    }

    #[test]
    fn strata_gives_nothing_for_two_columns_of_the_same_name_or_no_selection() {
        let root = strata_columns(&["pick", "pick"], &["alpha.txt"]);
        assert_eq!(
            strata_selected_name(&root, Path::new("/tmp/pick/pick")),
            None
        );
        let root = strata_columns(&["pick"], &[]);
        assert_eq!(strata_selected_name(&root, Path::new("/tmp/pick")), None);
        let root = strata_columns(&["pick"], &["alpha.txt", "aether.png"]);
        assert_eq!(strata_selected_name(&root, Path::new("/tmp/pick")), None);
    }
}
