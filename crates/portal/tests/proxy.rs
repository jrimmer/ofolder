//! The proxy, end to end, on a private session bus with a fake real backend.
//! Nothing here touches the user's own bus or portal configuration.

use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use ofolder_portal::hooks::{Call, Hooks, Options, response};
use ofolder_portal::{PORTAL_PATH, preseed, serve};
use zbus::connection::Builder;
use zbus::zvariant::{ObjectPath, OwnedValue, Value};
use zbus::{Connection, Proxy, fdo, interface};

const PROXY_NAME: &str = "org.freedesktop.impl.portal.desktop.ofolder";
const FAKE_NAME: &str = "org.freedesktop.impl.portal.desktop.fake";
const FILE_CHOOSER: &str = "org.freedesktop.impl.portal.FileChooser";
const REQUEST: &str = "org.freedesktop.impl.portal.Request";
const HANDLE: &str = "/org/freedesktop/portal/desktop/request/test/one";

struct PrivateBus {
    child: Child,
    address: String,
}

impl PrivateBus {
    fn start() -> Self {
        let mut child = Command::new("dbus-daemon")
            .args(["--session", "--nofork", "--print-address"])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("dbus-daemon must be installed to run these tests");
        let mut line = String::new();
        BufReader::new(child.stdout.take().expect("piped"))
            .read_line(&mut line)
            .expect("the daemon prints its address");
        Self {
            child,
            address: line.trim().to_owned(),
        }
    }

    fn builder(&self) -> Builder<'_> {
        Builder::address(self.address.as_str()).expect("valid address")
    }

    async fn connect(&self) -> Connection {
        self.builder().build().await.expect("connect")
    }
}

impl Drop for PrivateBus {
    fn drop(&mut self) {
        // The daemon is ours; either it is killed or it already exited.
        self.child.kill().ok();
        self.child.wait().ok();
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Seen {
    app_id: String,
    title: String,
    folder: Option<PathBuf>,
    name: Option<String>,
}

struct FakeBackend {
    seen: Arc<Mutex<Vec<Seen>>>,
    closed: Arc<AtomicBool>,
    /// Keep the dialog open until `Close` arrives.
    hold: bool,
}

struct FakeRequest {
    closed: Arc<AtomicBool>,
}

#[interface(name = "org.freedesktop.impl.portal.Request")]
impl FakeRequest {
    fn close(&self) {
        self.closed.store(true, Ordering::SeqCst);
    }
}

#[interface(name = "org.freedesktop.impl.portal.FileChooser")]
impl FakeBackend {
    async fn save_file(
        &self,
        #[zbus(connection)] connection: &Connection,
        handle: ObjectPath<'_>,
        app_id: &str,
        _parent_window: &str,
        title: &str,
        options: Options,
    ) -> fdo::Result<(u32, Options)> {
        self.seen.lock().expect("lock").push(Seen {
            app_id: app_id.to_owned(),
            title: title.to_owned(),
            folder: preseed::current_folder(&options),
            name: preseed::current_name(&options),
        });
        connection
            .object_server()
            .at(
                &handle,
                FakeRequest {
                    closed: self.closed.clone(),
                },
            )
            .await?;
        if self.hold {
            while !self.closed.load(Ordering::SeqCst) {
                async_io::Timer::after(Duration::from_millis(10)).await;
            }
            return Ok((response::CANCELLED, Options::new()));
        }
        let mut results = Options::new();
        results.insert(
            "uris".to_owned(),
            OwnedValue::try_from(Value::from(vec!["file:///tmp/saved.txt"])).expect("uris"),
        );
        Ok((response::SUCCESS, results))
    }
}

struct DroppedFlag(Arc<AtomicBool>);

impl Drop for DroppedFlag {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

#[derive(Default)]
struct Recorder {
    seed: Option<PathBuf>,
    begun: Mutex<Vec<String>>,
    ended: Mutex<Vec<(String, u32)>>,
    guard_dropped: Arc<AtomicBool>,
}

impl Hooks for Recorder {
    fn begin(&self, call: &Call, options: &mut Options) -> Box<dyn Send> {
        self.begun.lock().expect("lock").push(call.app_id.clone());
        if let Some(seed) = &self.seed {
            preseed::set_current_folder(options, seed);
        }
        Box::new(DroppedFlag(self.guard_dropped.clone()))
    }

    fn end(&self, call: &Call, response: u32, _results: &Options) {
        self.ended
            .lock()
            .expect("lock")
            .push((call.app_id.clone(), response));
    }
}

struct Rig {
    bus: PrivateBus,
    seen: Arc<Mutex<Vec<Seen>>>,
    closed: Arc<AtomicBool>,
    // Held so the services stay up for the test's duration.
    _services: Vec<Connection>,
}

async fn rig(hooks: Arc<dyn Hooks>, hold: bool, backend_name: &str) -> Rig {
    let bus = PrivateBus::start();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let closed = Arc::new(AtomicBool::new(false));
    let fake = bus
        .builder()
        .serve_at(
            PORTAL_PATH,
            FakeBackend {
                seen: seen.clone(),
                closed: closed.clone(),
                hold,
            },
        )
        .expect("serve fake")
        .name(FAKE_NAME)
        .expect("name")
        .build()
        .await
        .expect("fake backend");
    let proxy = serve(bus.builder(), PROXY_NAME, backend_name, hooks)
        .await
        .expect("proxy");
    Rig {
        bus,
        seen,
        closed,
        _services: vec![fake, proxy],
    }
}

fn request_options(folder: &str, name: &str) -> Options {
    let mut options = Options::new();
    preseed::set_current_folder(&mut options, std::path::Path::new(folder));
    options.insert(
        "current_name".to_owned(),
        OwnedValue::try_from(Value::from(name)).expect("name"),
    );
    options
}

async fn save_file(client: &Connection, options: Options) -> zbus::Result<(u32, Options)> {
    let proxy = Proxy::new(client, PROXY_NAME, PORTAL_PATH, FILE_CHOOSER).await?;
    let handle = ObjectPath::try_from(HANDLE).expect("handle");
    proxy
        .call(
            "SaveFile",
            &(handle, "org.example.App", "", "Save as", &options),
        )
        .await
}

#[test]
fn a_request_is_forwarded_and_the_answer_comes_back_unchanged() {
    zbus::block_on(async {
        let rig = rig(
            Arc::new(ofolder_portal::hooks::PassThrough),
            false,
            FAKE_NAME,
        )
        .await;
        let client = rig.bus.connect().await;
        let (code, results) = save_file(&client, request_options("/orig", "a.txt"))
            .await
            .expect("call");
        assert_eq!(code, response::SUCCESS);
        let uris =
            Vec::<String>::try_from(results["uris"].try_clone().expect("clone")).expect("uris");
        assert_eq!(uris, ["file:///tmp/saved.txt"]);
        assert_eq!(
            rig.seen.lock().expect("lock").as_slice(),
            [Seen {
                app_id: "org.example.App".to_owned(),
                title: "Save as".to_owned(),
                folder: Some(PathBuf::from("/orig")),
                name: Some("a.txt".to_owned()),
            }]
        );
    });
}

#[test]
fn a_hook_can_change_where_the_dialog_starts_before_it_sees_the_request() {
    zbus::block_on(async {
        let hooks = Arc::new(Recorder {
            seed: Some(PathBuf::from("/seeded")),
            ..Recorder::default()
        });
        let rig = rig(hooks, false, FAKE_NAME).await;
        let client = rig.bus.connect().await;
        save_file(&client, request_options("/orig", "a.txt"))
            .await
            .expect("call");
        let seen = rig.seen.lock().expect("lock");
        assert_eq!(seen[0].folder, Some(PathBuf::from("/seeded")));
        assert_eq!(
            seen[0].name.as_deref(),
            Some("a.txt"),
            "other options pass through"
        );
    });
}

#[test]
fn the_end_hook_sees_the_answer_and_the_guard_is_dropped_when_the_dialog_is_done() {
    zbus::block_on(async {
        let hooks = Arc::new(Recorder::default());
        let rig = rig(hooks.clone(), false, FAKE_NAME).await;
        let client = rig.bus.connect().await;
        save_file(&client, request_options("/orig", "a.txt"))
            .await
            .expect("call");
        assert_eq!(
            hooks.begun.lock().expect("lock").as_slice(),
            ["org.example.App"]
        );
        assert_eq!(
            hooks.ended.lock().expect("lock").as_slice(),
            [("org.example.App".to_owned(), response::SUCCESS)]
        );
        assert!(
            hooks.guard_dropped.load(Ordering::SeqCst),
            "the strip's guard must not outlive the dialog"
        );
    });
}

#[test]
fn closing_the_request_reaches_the_dialog_the_real_backend_opened() {
    zbus::block_on(async {
        let rig = rig(
            Arc::new(ofolder_portal::hooks::PassThrough),
            true,
            FAKE_NAME,
        )
        .await;
        let client = rig.bus.connect().await;
        let caller = {
            let client = client.clone();
            std::thread::spawn(move || {
                zbus::block_on(async {
                    save_file(&client, request_options("/orig", "a.txt")).await
                })
            })
        };
        // Wait until the real dialog is open, then cancel through the proxy.
        while rig.seen.lock().expect("lock").is_empty() {
            async_io::Timer::after(Duration::from_millis(10)).await;
        }
        let request = Proxy::new(&client, PROXY_NAME, HANDLE, REQUEST)
            .await
            .expect("request proxy");
        request.call::<_, _, ()>("Close", &()).await.expect("close");
        let (code, _) = caller
            .join()
            .expect("thread")
            .expect("the call still answers");
        assert_eq!(code, response::CANCELLED);
        assert!(
            rig.closed.load(Ordering::SeqCst),
            "the real dialog must have been told"
        );
    });
}

#[test]
fn a_backend_that_cannot_be_reached_is_answered_as_other_and_not_as_a_crash() {
    zbus::block_on(async {
        let hooks = Arc::new(Recorder::default());
        let rig = rig(hooks.clone(), false, "org.example.NoSuchBackend").await;
        let client = rig.bus.connect().await;
        let (code, results) = save_file(&client, request_options("/orig", "a.txt"))
            .await
            .expect("call");
        assert_eq!(code, response::OTHER);
        assert!(results.is_empty());
        assert_eq!(
            hooks.ended.lock().expect("lock").as_slice(),
            [("org.example.App".to_owned(), response::OTHER)]
        );
        assert!(hooks.guard_dropped.load(Ordering::SeqCst));
    });
}

#[test]
fn the_request_object_is_removed_when_the_dialog_is_done_so_the_path_can_be_reused() {
    zbus::block_on(async {
        let rig = rig(
            Arc::new(ofolder_portal::hooks::PassThrough),
            false,
            FAKE_NAME,
        )
        .await;
        let client = rig.bus.connect().await;
        // Two calls on the same handle path, one after the other: the second
        // fails if the first left its request object behind.
        let first = save_file(&client, request_options("/a", "a.txt")).await;
        // The fake backend registers its own request at the handle too; clear it.
        rig.closed.store(false, Ordering::SeqCst);
        let second = save_file(&client, request_options("/b", "b.txt")).await;
        assert!(first.is_ok());
        assert!(second.is_ok(), "{second:?}");
    });
}
