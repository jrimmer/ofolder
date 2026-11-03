//! Flea's picker.
//!
//! Flea's portal backend runs `flea --pick <reply>` once per request, with the
//! request as JSON in `FLEA_PICKER`. Neither is documented; both were read
//! from `/usr/lib/flea/flea-portal` and may change in a Flea update. The
//! folder on show is the directory its `--backend` helper holds open.

use std::cell::RefCell;
use std::fs;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread::sleep;
use std::time::{Duration, Instant};

use ofolder_core::adapter::{
    Adapter, AdapterError, Capabilities, DialogRequest, WindowRef, known_capabilities,
};
use ofolder_core::readers::flea_folder;
use ofolder_core::system::{Compositor, Typist};
use serde_json::json;

use crate::guard::KeyGuard;
use crate::procfs::ProcFs;
use crate::selection::{PickerIpc, QsIpc, flea_candidate, flea_snapshot_folder};

/// The window class of Flea's picker.
pub const PICKER_CLASS: &str = "com.thisisgm.flea.picker";

/// The request in the shape the picker reads: the same keys the portal
/// backend fills in, with the ones a plain save needs left empty.
pub fn picker_request_json(request: &DialogRequest) -> Result<String, AdapterError> {
    let folder = request
        .folder
        .to_str()
        .ok_or_else(|| AdapterError::Refused("the folder is not valid UTF-8".to_owned()))?;
    Ok(json!({
        "mode": "save",
        "title": request.title,
        "app": "ofolder",
        "accept": "",
        "multiple": false,
        "directory": false,
        "folder": folder,
        "file": "",
        "name": request.suggested_name,
        "files": [],
        "filters": [],
        "current": "",
        "currentIndex": -1,
    })
    .to_string())
}

/// Asks the kernel to send SIGTERM to the picker if ofolder dies, however it
/// dies, so a crash or a kill cannot leave an orphaned dialog behind.
fn die_with_parent() -> std::io::Result<()> {
    // SAFETY: `prctl` with `PR_SET_PDEATHSIG` only sets a flag on the calling
    // process. It is async-signal-safe, which is what `pre_exec` requires.
    let rc = unsafe { libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM as libc::c_ulong) };
    if rc == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

/// The pid of a picker window that was not there before: the newest of
/// those whose pid is not in `known`. The portal frontend starts Flea's
/// picker itself, so the pid is found by looking for the window.
pub fn find_new_picker(compositor: &dyn Compositor, known: &[u32]) -> Option<u32> {
    compositor
        .clients()
        .ok()?
        .into_iter()
        .filter(|c| c.class == PICKER_CLASS && !known.contains(&c.pid))
        .map(|c| c.pid)
        .max()
}

/// The pids of every picker window open now.
pub fn picker_pids(compositor: &dyn Compositor) -> Vec<u32> {
    compositor
        .clients()
        .map(|cs| {
            cs.into_iter()
                .filter(|c| c.class == PICKER_CLASS)
                .map(|c| c.pid)
                .collect()
        })
        .unwrap_or_default()
}

pub struct FleaAdapter {
    compositor: Arc<dyn Compositor>,
    guard: KeyGuard,
    procfs: ProcFs,
    flea_binary: PathBuf,
    /// Flea's own state folders, which its helper also holds open.
    ignore_under: Vec<PathBuf>,
    child: RefCell<Option<Child>>,
    reply_dir: Option<PathBuf>,
    /// A picker this adapter did not start, for tests and adoption.
    adopted_pid: Option<u32>,
    /// The picker's read-only IPC, for the folder and the selected item.
    ipc: Arc<dyn PickerIpc>,
    /// The last snapshot and when it was taken, so the folder and the
    /// selection read in one poll share one call.
    snapshot: Mutex<Option<(u32, Instant, Option<String>)>>,
}

/// How long a snapshot answers for both the folder and the selection. The
/// strip polls every 200 ms, so one poll reuses its own call and no other.
const SNAPSHOT_FRESH: Duration = Duration::from_millis(120);

/// The folder from the picker's IPC and the folder from the helper's open
/// descriptors, made into one answer. Either alone is used; when both answer
/// and disagree, neither is trusted and the folder is unknown.
pub fn reconcile_folder(ipc: Option<PathBuf>, fd: Option<PathBuf>) -> Option<PathBuf> {
    match (ipc, fd) {
        (Some(a), Some(b)) => same_folder(&a, &b).then_some(a),
        (a, b) => a.or(b),
    }
}

fn same_folder(a: &Path, b: &Path) -> bool {
    a == b
        || matches!(
            (fs::canonicalize(a), fs::canonicalize(b)),
            (Ok(x), Ok(y)) if x == y
        )
}

impl FleaAdapter {
    pub fn new(compositor: Arc<dyn Compositor>, typist: Arc<dyn Typist>) -> Self {
        let home = std::env::var_os("HOME").map(PathBuf::from);
        let mut ignore_under: Vec<PathBuf> = home
            .iter()
            .flat_map(|h| {
                [
                    ".cache",
                    ".local/state",
                    ".local/share/flea",
                    ".config/flea",
                ]
                .map(|rel| h.join(rel))
            })
            .collect();
        ignore_under.extend(std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from));
        Self {
            guard: KeyGuard::new(compositor.clone(), typist, Duration::from_millis(200)),
            compositor,
            procfs: ProcFs::default(),
            flea_binary: PathBuf::from("flea"),
            ignore_under,
            child: RefCell::new(None),
            reply_dir: None,
            adopted_pid: None,
            ipc: Arc::new(QsIpc),
            snapshot: Mutex::new(None),
        }
    }

    #[must_use]
    pub fn with_pace(mut self, pace: Duration) -> Self {
        self.guard.set_pace(pace);
        self
    }

    #[must_use]
    pub fn with_procfs(mut self, procfs: ProcFs) -> Self {
        self.procfs = procfs;
        self
    }

    #[must_use]
    pub fn with_binary(mut self, binary: impl Into<PathBuf>) -> Self {
        self.flea_binary = binary.into();
        self
    }

    #[must_use]
    pub fn with_ignored(mut self, ignore_under: Vec<PathBuf>) -> Self {
        self.ignore_under = ignore_under;
        self
    }

    /// Treats a picker already running as pid `pid` as this adapter's own.
    #[must_use]
    pub fn with_ipc(mut self, ipc: Arc<dyn PickerIpc>) -> Self {
        self.ipc = ipc;
        self
    }

    #[must_use]
    pub fn adopting(mut self, pid: u32) -> Self {
        self.adopted_pid = Some(pid);
        self
    }

    fn picker_pid(&self) -> Option<u32> {
        self.child
            .borrow()
            .as_ref()
            .map(Child::id)
            .or(self.adopted_pid)
    }

    /// The picker's snapshot, reused within [`SNAPSHOT_FRESH`].
    fn snapshot(&self, pid: u32) -> Option<String> {
        if let Ok(cache) = self.snapshot.lock()
            && let Some((cached_pid, at, answer)) = cache.as_ref()
            && *cached_pid == pid
            && at.elapsed() < SNAPSHOT_FRESH
        {
            return answer.clone();
        }
        let answer = self.ipc.snapshot(pid);
        if let Ok(mut cache) = self.snapshot.lock() {
            *cache = Some((pid, Instant::now(), answer.clone()));
        }
        answer
    }

    /// The folder Flea's `--backend` helpers hold open, read from `/proc`.
    fn descriptor_folder(&self, pid: u32) -> Option<PathBuf> {
        let mut tree = self.procfs.descendants(pid);
        tree.insert(0, pid);
        let open: Vec<PathBuf> = tree
            .into_iter()
            .filter(|p| {
                self.procfs
                    .cmdline(*p)
                    .is_some_and(|c| c.contains("--backend"))
            })
            .flat_map(|p| self.procfs.open_dirs(p))
            .collect();
        flea_folder(&open, &self.ignore_under)
    }

    fn kill(&self) {
        if let Some(child) = self.child.borrow_mut().as_mut() {
            // The process may already be gone; either way it is not running.
            child.kill().ok();
            child.wait().ok();
        }
    }
}

impl Drop for FleaAdapter {
    fn drop(&mut self) {
        self.kill();
        if let Some(dir) = &self.reply_dir {
            // Cleanup only: a leftover reply folder must not turn a clean exit into a failure.
            fs::remove_dir_all(dir).ok();
        }
    }
}

impl Adapter for FleaAdapter {
    fn id(&self) -> &'static str {
        "flea"
    }

    fn capabilities(&self) -> Capabilities {
        known_capabilities("flea")
    }

    fn launch(&mut self, request: &DialogRequest) -> Result<(), AdapterError> {
        if self.picker_pid().is_some() {
            return Err(AdapterError::Refused(
                "a picker is already running".to_owned(),
            ));
        }
        let json = picker_request_json(request)?;
        let base =
            std::env::var_os("XDG_RUNTIME_DIR").map_or_else(std::env::temp_dir, PathBuf::from);
        let dir = base.join(format!("ofolder-flea-{}", std::process::id()));
        fs::create_dir_all(&dir)
            .map_err(|e| AdapterError::Backend(format!("reply folder: {e}")))?;
        let mut command = Command::new(&self.flea_binary);
        command
            .arg("--pick")
            .arg(dir.join("reply.json"))
            .env("FLEA_PICKER", json)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        // SAFETY: the closure passed to `pre_exec` runs between fork and exec
        // and only calls `die_with_parent`, which is async-signal-safe. The
        // parent-death signal is tied to the thread that spawned the child,
        // so `launch` must be called from a thread that outlives the picker.
        unsafe { command.pre_exec(die_with_parent) };
        let child = command
            .spawn()
            .map_err(|e| AdapterError::Backend(format!("could not start flea: {e}")))?;
        self.reply_dir = Some(dir);
        *self.child.borrow_mut() = Some(child);
        Ok(())
    }

    fn window(&self) -> Option<WindowRef> {
        let pid = self.picker_pid()?;
        let clients = self.compositor.clients().ok()?;
        clients
            .into_iter()
            .find(|c| c.class == PICKER_CLASS && c.pid == pid)
            .map(|c| WindowRef {
                address: c.address,
                pid: c.pid,
                class: c.class,
                title: c.title,
            })
    }

    /// The picker's own answer first, the helper's open folders as the
    /// fallback; see [`reconcile_folder`].
    fn read_folder(&self) -> Option<PathBuf> {
        let pid = self.picker_pid().filter(|_| self.alive())?;
        let ipc = self
            .snapshot(pid)
            .and_then(|snap| flea_snapshot_folder(&snap));
        reconcile_folder(ipc, self.descriptor_folder(pid))
    }

    fn goto(&mut self, folder: &Path) -> Result<(), AdapterError> {
        self.guard
            .type_path(&|| self.window().map(|w| w.address), folder)
    }

    fn close(&mut self) -> Result<(), AdapterError> {
        for _ in 0..3 {
            if !self.alive() {
                return Ok(());
            }
            if let Some(window) = self.window() {
                // Refused when the picker is not the active window; then it is killed below.
                self.guard.escape(&window.address).ok();
            }
            sleep(self.guard.pace() * 3);
        }
        if self.alive() {
            self.kill();
        }
        Ok(())
    }

    fn alive(&self) -> bool {
        match self.child.borrow_mut().as_mut() {
            Some(child) => matches!(child.try_wait(), Ok(None)),
            // A picker the portal frontend started: alive while that process
            // runs and is still a picker, so a reused pid does not count.
            None => self.adopted_pid.is_some_and(|pid| {
                self.procfs.is_running(pid)
                    && self
                        .procfs
                        .cmdline(pid)
                        .is_some_and(|c| c.contains("picker"))
            }),
        }
    }

    /// From the picker's own read-only `snapshot` call, which answers the
    /// folder, the cursor row and the marks at one instant.
    fn selection(&self) -> Option<PathBuf> {
        let pid = self.picker_pid().filter(|_| self.alive())?;
        let candidate = flea_candidate(&self.snapshot(pid)?)?;
        candidate.symlink_metadata().ok().map(|_| candidate)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use ofolder_core::hypr::{Client, Monitor, parse_clients};
    use ofolder_core::system::{Keys, SystemError};

    use super::*;
    use crate::procfs::testing::{TempDir, add_process};

    const PID: u32 = 4242;

    fn client(address: &str, class: &str, pid: u32) -> Client {
        let json = format!(
            r#"[{{"address":"{address}","class":"{class}","title":"Save","pid":{pid},
            "at":[0,0],"size":[10,10],"floating":true,"workspace":{{"id":1,"name":"1"}},"monitor":0}}]"#
        );
        parse_clients(&json).expect("sample is valid").remove(0)
    }

    struct FakeCompositor {
        clients: Vec<Client>,
        active: Mutex<Option<String>>,
        /// Whether asking for focus actually moves it.
        focus_works: bool,
        focused: Mutex<Vec<String>>,
    }

    impl FakeCompositor {
        fn new(clients: Vec<Client>, active: Option<&str>, focus_works: bool) -> Arc<Self> {
            Arc::new(Self {
                clients,
                active: Mutex::new(active.map(str::to_owned)),
                focus_works,
                focused: Mutex::new(Vec::new()),
            })
        }
    }

    impl Compositor for FakeCompositor {
        fn clients(&self) -> Result<Vec<Client>, SystemError> {
            Ok(self.clients.clone())
        }
        fn monitors(&self) -> Result<Vec<Monitor>, SystemError> {
            Ok(Vec::new())
        }
        fn active_window(&self) -> Result<Option<String>, SystemError> {
            Ok(self.active.lock().expect("lock").clone())
        }
        fn focus(&self, address: &str) -> Result<(), SystemError> {
            self.focused.lock().expect("lock").push(address.to_owned());
            if self.focus_works {
                *self.active.lock().expect("lock") = Some(address.to_owned());
            }
            Ok(())
        }
    }

    struct FakeTypist {
        sent: Mutex<Vec<Keys>>,
        compositor: Arc<FakeCompositor>,
        /// After this many keys, another window takes focus.
        steal_focus_after: Option<usize>,
    }

    impl Typist for FakeTypist {
        fn send(&self, keys: &Keys) -> Result<(), SystemError> {
            let mut sent = self.sent.lock().expect("lock");
            sent.push(keys.clone());
            if self.steal_focus_after == Some(sent.len()) {
                *self.compositor.active.lock().expect("lock") = Some("0xdead".to_owned());
            }
            Ok(())
        }
    }

    struct Rig {
        adapter: FleaAdapter,
        compositor: Arc<FakeCompositor>,
        typist: Arc<FakeTypist>,
    }

    fn rig(active: Option<&str>, focus_works: bool, steal_focus_after: Option<usize>) -> Rig {
        let compositor = FakeCompositor::new(
            vec![
                client("0xaaa", PICKER_CLASS, PID),
                client("0xbbb", PICKER_CLASS, 9999),
            ],
            active,
            focus_works,
        );
        let typist = Arc::new(FakeTypist {
            sent: Mutex::new(Vec::new()),
            compositor: compositor.clone(),
            steal_focus_after,
        });
        let adapter = FleaAdapter::new(compositor.clone(), typist.clone())
            .with_pace(Duration::ZERO)
            .adopting(PID);
        Rig {
            adapter,
            compositor,
            typist,
        }
    }

    fn sent(rig: &Rig) -> Vec<Keys> {
        rig.typist.sent.lock().expect("lock").clone()
    }

    #[test]
    fn goto_refuses_to_type_when_the_picker_cannot_be_made_the_active_window() {
        let mut r = rig(Some("0xother"), false, None);
        let err = r
            .adapter
            .goto(Path::new("/home/j/Work"))
            .expect_err("must refuse");
        assert!(matches!(err, AdapterError::Refused(_)), "{err:?}");
        assert!(
            sent(&r).is_empty(),
            "nothing may be typed into another window"
        );
    }

    #[test]
    fn goto_focuses_the_picker_then_sends_ctrl_l_the_path_and_return_in_order() {
        let mut r = rig(Some("0xother"), true, None);
        r.adapter.goto(Path::new("/home/j/Work")).expect("goto");
        assert_eq!(
            r.compositor.focused.lock().expect("lock").as_slice(),
            ["0xaaa"]
        );
        assert_eq!(
            sent(&r),
            [
                Keys::CtrlL,
                Keys::Text("/home/j/Work".to_owned()),
                Keys::Return
            ]
        );
    }

    #[test]
    fn goto_stops_the_moment_focus_leaves_the_picker() {
        let mut r = rig(Some("0xaaa"), true, Some(1));
        let err = r
            .adapter
            .goto(Path::new("/home/j/Work"))
            .expect_err("must stop");
        assert!(matches!(err, AdapterError::Refused(_)), "{err:?}");
        assert_eq!(
            sent(&r),
            [Keys::CtrlL],
            "the path must not follow into another window"
        );
    }

    #[test]
    fn goto_refuses_a_path_that_would_act_as_a_key_before_touching_focus() {
        let mut r = rig(Some("0xaaa"), true, None);
        let err = r
            .adapter
            .goto(Path::new("/home/j/a\nb"))
            .expect_err("must refuse");
        assert!(matches!(err, AdapterError::Refused(_)), "{err:?}");
        assert!(sent(&r).is_empty());
        assert!(r.compositor.focused.lock().expect("lock").is_empty());
    }

    #[test]
    fn goto_reports_not_running_when_the_picker_has_no_window() {
        let compositor = FakeCompositor::new(Vec::new(), None, true);
        let typist = Arc::new(FakeTypist {
            sent: Mutex::new(Vec::new()),
            compositor: compositor.clone(),
            steal_focus_after: None,
        });
        let mut adapter = FleaAdapter::new(compositor, typist)
            .with_pace(Duration::ZERO)
            .adopting(PID);
        let err = adapter.goto(Path::new("/home/j")).expect_err("no window");
        assert!(matches!(err, AdapterError::NotRunning), "{err:?}");
    }

    #[test]
    fn the_window_is_matched_by_pid_because_addresses_and_classes_repeat() {
        let r = rig(None, true, None);
        let window = r.adapter.window().expect("a window");
        assert_eq!((window.address.as_str(), window.pid), ("0xaaa", PID));
    }

    #[test]
    fn the_folder_comes_from_the_backend_helpers_open_directory() {
        let tmp = TempDir::new("flea-folder");
        let (cwd, shown) = (tmp.0.join("cwd"), tmp.0.join("shown"));
        fs::create_dir_all(&cwd).expect("dir");
        fs::create_dir_all(&shown).expect("dir");
        add_process(
            &tmp.0,
            PID,
            1,
            "qs -p /usr/share/flea/ui/boot/picker.qml",
            &cwd,
            &[],
        );
        add_process(
            &tmp.0,
            4243,
            PID,
            "/usr/bin/flea --backend",
            &cwd,
            &[&shown],
        );
        add_process(&tmp.0, 4244, PID, "/usr/bin/flea --backend", &cwd, &[]);
        let r = rig(None, true, None);
        let adapter = r
            .adapter
            .with_procfs(ProcFs::with_root(&tmp.0))
            .with_ignored(Vec::new());
        assert_eq!(adapter.read_folder(), Some(shown));
    }

    #[test]
    fn the_folder_ignores_directories_held_open_by_other_processes_in_the_tree() {
        let tmp = TempDir::new("flea-other");
        let (cwd, other) = (tmp.0.join("cwd"), tmp.0.join("other"));
        fs::create_dir_all(&cwd).expect("dir");
        fs::create_dir_all(&other).expect("dir");
        add_process(&tmp.0, PID, 1, "qs -p picker.qml", &cwd, &[&other]);
        let r = rig(None, true, None);
        let adapter = r
            .adapter
            .with_procfs(ProcFs::with_root(&tmp.0))
            .with_ignored(Vec::new());
        assert_eq!(adapter.read_folder(), None);
    }

    #[test]
    fn flea_state_folders_are_not_mistaken_for_the_folder_shown() {
        let tmp = TempDir::new("flea-state");
        let (cwd, shown, state) = (tmp.0.join("cwd"), tmp.0.join("shown"), tmp.0.join("state"));
        for d in [&cwd, &shown, &state] {
            fs::create_dir_all(d).expect("dir");
        }
        add_process(&tmp.0, PID, 1, "qs -p picker.qml", &cwd, &[]);
        add_process(&tmp.0, 4243, PID, "flea --backend", &cwd, &[&state, &shown]);
        let r = rig(None, true, None);
        let adapter = r
            .adapter
            .with_procfs(ProcFs::with_root(&tmp.0))
            .with_ignored(vec![state]);
        assert_eq!(adapter.read_folder(), Some(shown));
    }

    #[test]
    fn a_dialog_showing_the_helpers_own_working_directory_still_reads_back() {
        // Flea's helper runs from the home folder, and home is a folder a dialog shows.
        let tmp = TempDir::new("flea-cwd-shown");
        let home = tmp.0.join("home");
        fs::create_dir_all(&home).expect("dir");
        add_process(&tmp.0, PID, 1, "qs -p picker.qml", &home, &[]);
        add_process(&tmp.0, 4243, PID, "flea --backend", &home, &[&home]);
        let r = rig(None, true, None);
        let adapter = r.adapter.with_procfs(ProcFs::with_root(&tmp.0));
        assert_eq!(adapter.read_folder(), Some(home));
    }

    #[test]
    fn a_picker_that_was_not_open_before_is_found_and_the_known_ones_are_not() {
        let r = rig(None, true, None);
        assert_eq!(
            find_new_picker(&*r.compositor, &[]),
            Some(9999),
            "the newest of the two"
        );
        assert_eq!(find_new_picker(&*r.compositor, &[9999]), Some(PID));
        assert_eq!(find_new_picker(&*r.compositor, &[PID, 9999]), None);
        let mut pids = picker_pids(&*r.compositor);
        pids.sort_unstable();
        assert_eq!(pids, [PID, 9999]);
    }

    struct FakeIpc(Option<String>);

    impl PickerIpc for FakeIpc {
        fn snapshot(&self, pid: u32) -> Option<String> {
            assert_eq!(pid, PID, "the call goes to the picker's own process");
            self.0.clone()
        }
    }

    #[test]
    fn the_selection_is_the_cursor_row_the_picker_reports_when_it_exists() {
        let tmp = TempDir::new("flea-selection");
        fs::write(tmp.0.join("alpha.txt"), "x").expect("file");
        let procs = TempDir::new("flea-selection-proc");
        add_process(&procs.0, PID, 1, "qs -p picker.qml", &tmp.0, &[]);
        let snap =
            |name: &str| Some(json!({"path": tmp.0, "cursorName": name, "marks": []}).to_string());
        let picker = |ipc: FakeIpc| {
            rig(None, true, None)
                .adapter
                .with_procfs(ProcFs::with_root(&procs.0))
                .with_ipc(Arc::new(ipc))
        };
        let adapter = picker(FakeIpc(snap("alpha.txt")));
        assert_eq!(adapter.selection(), Some(tmp.0.join("alpha.txt")));
        let gone = picker(FakeIpc(snap("deleted.txt")));
        assert_eq!(gone.selection(), None);
        let silent = picker(FakeIpc(None));
        assert_eq!(silent.selection(), None);
    }

    /// Answers the snapshot calls and counts them.
    struct CountingIpc {
        answer: Option<String>,
        calls: std::sync::atomic::AtomicUsize,
    }

    impl PickerIpc for CountingIpc {
        fn snapshot(&self, _pid: u32) -> Option<String> {
            self.calls
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            self.answer.clone()
        }
    }

    /// A picker whose helper holds `held` open and whose IPC answers `ipc_path`.
    fn folder_rig(
        procs: &Path,
        held: Option<&Path>,
        ipc_path: Option<&Path>,
    ) -> (FleaAdapter, Arc<CountingIpc>) {
        let cwd = procs.join("cwd");
        fs::create_dir_all(&cwd).expect("dir");
        add_process(procs, PID, 1, "qs -p picker.qml", &cwd, &[]);
        let open: Vec<&Path> = held.into_iter().collect();
        add_process(procs, 4243, PID, "flea --backend", &cwd, &open);
        let ipc = Arc::new(CountingIpc {
            answer: ipc_path.map(|p| json!({"path": p, "cursorName": "", "marks": []}).to_string()),
            calls: std::sync::atomic::AtomicUsize::new(0),
        });
        let adapter = rig(None, true, None)
            .adapter
            .with_procfs(ProcFs::with_root(procs))
            .with_ignored(Vec::new())
            .with_ipc(ipc.clone());
        (adapter, ipc)
    }

    #[test]
    fn the_folder_comes_from_the_picker_when_both_ways_agree_or_only_it_answers() {
        let tmp = TempDir::new("flea-ipc-agree");
        let shown = tmp.0.join("shown");
        fs::create_dir_all(&shown).expect("dir");
        let (both, _) = folder_rig(&tmp.0, Some(&shown), Some(&shown));
        assert_eq!(both.read_folder(), Some(shown.clone()));
        let only = TempDir::new("flea-ipc-only");
        let there = only.0.join("there");
        fs::create_dir_all(&there).expect("dir");
        // A folder under /tmp where the helper holds nothing open.
        let (ipc_only, _) = folder_rig(&only.0, None, Some(&there));
        assert_eq!(ipc_only.read_folder(), Some(there));
    }

    #[test]
    fn the_descriptors_answer_when_the_picker_does_not() {
        let tmp = TempDir::new("flea-fd-only");
        let shown = tmp.0.join("shown");
        fs::create_dir_all(&shown).expect("dir");
        let (silent, _) = folder_rig(&tmp.0, Some(&shown), None);
        assert_eq!(silent.read_folder(), Some(shown.clone()));
        // The Recent view, or a folder that has gone, is no answer either.
        let other = TempDir::new("flea-fd-stale");
        let held = other.0.join("held");
        fs::create_dir_all(&held).expect("dir");
        let (stale, _) = folder_rig(&other.0, Some(&held), Some(&other.0.join("gone")));
        assert_eq!(stale.read_folder(), Some(held));
    }

    #[test]
    fn when_the_picker_and_the_descriptors_disagree_the_folder_is_unknown() {
        let tmp = TempDir::new("flea-disagree");
        let (a, b) = (tmp.0.join("a"), tmp.0.join("b"));
        fs::create_dir_all(&a).expect("dir");
        fs::create_dir_all(&b).expect("dir");
        let (adapter, _) = folder_rig(&tmp.0, Some(&a), Some(&b));
        assert_eq!(adapter.read_folder(), None);
        assert_eq!(reconcile_folder(None, None), None);
    }

    #[test]
    fn one_poll_asks_the_picker_once_for_the_folder_and_the_selection() {
        let tmp = TempDir::new("flea-one-call");
        let shown = tmp.0.join("shown");
        fs::create_dir_all(&shown).expect("dir");
        let (adapter, ipc) = folder_rig(&tmp.0, Some(&shown), Some(&shown));
        adapter.read_folder();
        adapter.selection();
        assert_eq!(ipc.calls.load(std::sync::atomic::Ordering::Relaxed), 1);
        sleep(SNAPSHOT_FRESH + Duration::from_millis(30));
        adapter.read_folder();
        assert_eq!(ipc.calls.load(std::sync::atomic::Ordering::Relaxed), 2);
    }

    #[test]
    fn an_adopted_picker_is_alive_only_while_its_process_runs_as_a_picker() {
        let tmp = TempDir::new("flea-alive");
        let adapter = |procs: &Path| {
            rig(None, true, None)
                .adapter
                .with_procfs(ProcFs::with_root(procs))
        };
        add_process(&tmp.0, PID, 1, "qs -p picker.qml", &tmp.0, &[]);
        assert!(adapter(&tmp.0).alive());
        // Exited but not yet reaped by the portal frontend.
        fs::write(
            tmp.0.join(PID.to_string()).join("stat"),
            format!("{PID} (qs) Z 1 1 1 0"),
        )
        .expect("stat");
        assert!(!adapter(&tmp.0).alive());
        // Gone, and then the pid reused by something that is not a picker.
        fs::remove_dir_all(tmp.0.join(PID.to_string())).expect("remove");
        assert!(!adapter(&tmp.0).alive());
        add_process(&tmp.0, PID, 1, "bash", &tmp.0, &[]);
        assert!(!adapter(&tmp.0).alive());
    }

    #[test]
    fn without_a_picker_the_folder_is_unknown() {
        let compositor = FakeCompositor::new(Vec::new(), None, true);
        let typist = Arc::new(FakeTypist {
            sent: Mutex::new(Vec::new()),
            compositor: compositor.clone(),
            steal_focus_after: None,
        });
        let adapter = FleaAdapter::new(compositor, typist);
        assert_eq!(adapter.read_folder(), None);
        assert!(!adapter.alive());
    }

    #[test]
    fn the_request_carries_the_keys_the_picker_reads() {
        let request = DialogRequest {
            folder: PathBuf::from("/home/j/Work"),
            suggested_name: "notes.txt".to_owned(),
            title: "Save as".to_owned(),
        };
        let json: serde_json::Value =
            serde_json::from_str(&picker_request_json(&request).expect("json")).expect("valid");
        assert_eq!(json["mode"], "save");
        assert_eq!(json["folder"], "/home/j/Work");
        assert_eq!(json["name"], "notes.txt");
        assert_eq!(json["filters"], serde_json::json!([]));
        assert_eq!(json["currentIndex"], -1);
    }

    #[test]
    fn launch_refuses_a_folder_that_is_not_utf8_instead_of_starting_in_the_wrong_place() {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;
        let mut r = rig(None, true, None);
        r.adapter.adopted_pid = None;
        let request = DialogRequest {
            folder: PathBuf::from(OsStr::from_bytes(b"/h/\xff")),
            suggested_name: String::new(),
            title: String::new(),
        };
        let err = r.adapter.launch(&request).expect_err("must refuse");
        assert!(matches!(err, AdapterError::Refused(_)), "{err:?}");
    }

    #[test]
    fn a_second_launch_is_refused_while_a_picker_is_running() {
        let mut r = rig(None, true, None);
        let request = DialogRequest {
            folder: PathBuf::from("/home/j"),
            suggested_name: String::new(),
            title: String::new(),
        };
        assert!(matches!(
            r.adapter.launch(&request),
            Err(AdapterError::Refused(_))
        ));
    }
}
