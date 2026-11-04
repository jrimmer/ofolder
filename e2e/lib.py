"""Shared pieces of the end-to-end suite.

Everything here runs ofolder against real dialogs inside a private Hyprland that
is nested in the desktop session. The nested session lives on a virtual output
so its window is drawn (a hidden window gets no frame callbacks, and Flea's
picker will not answer without them), and it has its own runtime directory,
home, D-Bus, portal frontend and clipboard. Nothing the tests do reaches the
real session's windows, keys, portals, settings or folders.

The one thing it touches on the host is the virtual output. `Session` removes it
on every exit path and then checks the real monitors are exactly as found.
"""
import atexit
import contextlib
import json
import os
import shutil
import signal
import subprocess
import sys
import tempfile
import time

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
OFOLDER = os.path.join(REPO, "target", "debug", "ofolder")
OUTPUT = "ofe2e-1"


def say(*a):
    print(*a, flush=True)


def run(cmd, env=None, timeout=20, check=False):
    try:
        r = subprocess.run(cmd, env=env, capture_output=True, text=True, timeout=timeout)
    except subprocess.TimeoutExpired:
        # Like a shell's `timeout`: the command is killed and reported as not having succeeded.
        return subprocess.CompletedProcess(cmd, 124, "", "timed out after %ss" % timeout)
    if check and r.returncode:
        raise RuntimeError("%s failed: %s" % (cmd, r.stderr.strip()))
    return r


def wait_for(limit, cond, step=0.1):
    """The value of `cond` once it is truthy, else None after `limit` seconds."""
    end = time.monotonic() + limit
    while time.monotonic() < end:
        v = cond()
        if v:
            return v
        time.sleep(step)
    return None


class Host:
    """The real session. Only ever read, apart from the one virtual output."""

    def __init__(self):
        self.rt = os.environ.get("XDG_RUNTIME_DIR") or "/run/user/%d" % os.getuid()
        sig = os.environ.get("HYPRLAND_INSTANCE_SIGNATURE") or self.only_instance()
        self.env = dict(os.environ, XDG_RUNTIME_DIR=self.rt, HYPRLAND_INSTANCE_SIGNATURE=sig)
        self.env.setdefault("WAYLAND_DISPLAY", "wayland-1")

    def only_instance(self):
        """The running Hyprland, when the caller's shell has not exported its signature."""
        r = run(["hyprctl", "instances", "-j"], env=dict(os.environ, XDG_RUNTIME_DIR=self.rt))
        try:
            found = [i for i in json.loads(r.stdout) if os.path.exists("/proc/%d" % i["pid"])]
        except (ValueError, KeyError):
            found = []
        if len(found) != 1:
            raise SystemExit(
                "cannot tell which Hyprland is the session (%d running); export HYPRLAND_INSTANCE_SIGNATURE" % len(found)
            )
        return found[0]["instance"]

    def hypr(self, *args):
        return run(["hyprctl", *args], env=self.env)

    def monitors(self):
        return json.loads(self.hypr("monitors", "-j").stdout)

    def real_layout(self):
        """The real monitors as (name, size, position, scale): what must survive a run."""
        return sorted(
            (m["name"], m["width"], m["height"], m["x"], m["y"], m["scale"])
            for m in self.monitors()
            if m["name"] != OUTPUT
        )

    def clients(self):
        return json.loads(self.hypr("clients", "-j").stdout)


class Session:
    """A nested Hyprland on a virtual output, torn down on every exit path."""

    def __init__(self, host):
        self.host = host
        self.dir = None
        self.rt = None
        self.proc = None
        self.env = None
        self.before = None
        self.output_made = False
        self.children = []

    # -- life cycle -------------------------------------------------------
    def __enter__(self):
        self.before = self.host.real_layout()
        if any(m["name"] == OUTPUT for m in self.host.monitors()):
            raise SystemExit(
                "a virtual output %s is already there (a run did not clean up). "
                "Run: e2e/run --cleanup" % OUTPUT
            )
        self.dir = tempfile.mkdtemp(prefix="ofe2e-state.")
        self.rt = tempfile.mkdtemp(prefix="ofe2e.")
        os.chmod(self.rt, 0o700)
        atexit.register(self.stop)
        for sig in (signal.SIGTERM, signal.SIGHUP):
            signal.signal(sig, lambda *_: sys.exit(130))
        self.host.hypr("output", "create", "headless", OUTPUT)
        self.output_made = True
        ws = wait_for(
            5,
            lambda: next(
                (m["activeWorkspace"]["name"] for m in self.host.monitors() if m["name"] == OUTPUT),
                None,
            ),
        )
        if not ws:
            raise SystemExit("the virtual output did not appear")
        home = os.path.join(self.dir, "home")
        os.makedirs(home)
        conf = os.path.join(self.dir, "hypr.lua")
        with open(conf, "w") as f:
            f.write('hl.monitor({ output = "", mode = "preferred", position = "auto", scale = 1 })\n')
            # Dialogs float at the size Omarchy gives them, as on the real desktop.
            for cls in ("com.thisisgm.flea.picker", "xdg-desktop-portal-gtk"):
                f.write(
                    'hl.window_rule({ name = "dialog-%s", match = { class = "^%s$" }, float = true, '
                    'size = "875 600", center = true })\n' % (cls.split(".")[-1], cls.replace(".", "\\\\."))
                )
        env = {
            "HOME": home,
            "PATH": os.environ["PATH"],
            "RUST_BACKTRACE": "1",
            "XDG_RUNTIME_DIR": self.rt,
            "WAYLAND_DISPLAY": os.path.join(self.host.rt, self.host.env["WAYLAND_DISPLAY"]),
            "XDG_CONFIG_HOME": home + "/.config",
            "XDG_DATA_HOME": home + "/.local/share",
            "XDG_STATE_HOME": home + "/.local/state",
            "XDG_CACHE_HOME": home + "/.cache",
        }
        log = open(os.path.join(self.dir, "hyprland.log"), "w")
        self.proc = subprocess.Popen(
            ["Hyprland", "-c", conf], env=env, stdout=log, stderr=subprocess.STDOUT, start_new_session=True
        )
        sock = wait_for(
            10,
            lambda: os.path.isdir(self.rt + "/hypr") and next(
                (d for d in os.listdir(self.rt + "/hypr") if os.path.exists(f"{self.rt}/hypr/{d}/.socket.sock")), None
            ),
        )
        if not sock:
            raise SystemExit("the nested Hyprland did not start; see %s/hyprland.log" % self.dir)
        self.env = dict(
            env,
            XDG_RUNTIME_DIR=self.rt,
            HYPRLAND_INSTANCE_SIGNATURE=sock,
            WAYLAND_DISPLAY="wayland-1",
            XDG_CURRENT_DESKTOP="Hyprland",
            XDG_SESSION_TYPE="wayland",
        )
        # Park the nested window on the virtual output without following it.
        addr = wait_for(
            10,
            lambda: next((c["address"] for c in self.host.clients() if c["pid"] == self.proc.pid), None),
        )
        if not addr:
            raise SystemExit("the nested window never showed on the host")
        self.host.hypr(
            "dispatch",
            'hl.dsp.window.move({ workspace = "%s", follow = false, window = "address:%s" })' % (ws, addr),
        )
        # Fill the virtual output, so the nested monitor is 1920x1080 and dialogs fit on it.
        self.host.hypr("dispatch", 'hl.dsp.window.fullscreen({ window = "address:%s" })' % addr)
        if not wait_for(5, lambda: self.hypr("monitors", "-j").returncode == 0):
            raise SystemExit("the nested Hyprland does not answer")
        size = wait_for(
            5,
            lambda: (lambda m: m if m and m[0]["width"] >= 1900 else None)(
                json.loads(self.hypr("monitors", "-j").stdout or "[]")
            ),
        )
        if not size:
            raise SystemExit("the nested monitor did not grow to the virtual output's size")
        return self

    def __exit__(self, *exc):
        self.stop()
        return False

    def stop(self):
        """Idempotent. Kills only processes that belong to this session's runtime directory."""
        if self.rt:
            self.kill_session_processes(compositor=True)
        if self.proc and self.proc.poll() is None:
            self.proc.terminate()
            with contextlib.suppress(subprocess.TimeoutExpired):
                self.proc.wait(5)
            if self.proc.poll() is None:
                self.proc.kill()
        if self.output_made:
            self.host.hypr("output", "remove", OUTPUT)
            self.output_made = False
            after = wait_for(5, lambda: self.host.real_layout() == self.before and self.host.real_layout())
            if self.host.real_layout() != self.before:
                say("WARNING: the real monitors differ from before the run:", self.before, "->", self.host.real_layout())
                self.layout_ok = False
            else:
                self.layout_ok = True
        for d in (self.rt, self.dir):
            if d and os.path.isdir(d):
                shutil.rmtree(d, ignore_errors=True)
        self.rt = self.dir = None

    def kill_session_processes(self, compositor=False):
        """Ends every process started inside this session. The nested compositor itself is spared
        unless `compositor` is true, so a later test class can use the session."""
        me = os.getpid()
        for sig in (signal.SIGTERM, signal.SIGKILL):
            found = False
            for pid in [int(p) for p in os.listdir("/proc") if p.isdigit()]:
                if pid == me or (not compositor and self.proc and pid == self.proc.pid):
                    continue
                try:
                    with open("/proc/%d/environ" % pid, "rb") as f:
                        env = f.read()
                except OSError:
                    continue
                if ("XDG_RUNTIME_DIR=%s\0" % self.rt).encode() in env:
                    found = True
                    with contextlib.suppress(ProcessLookupError):
                        os.kill(pid, sig)
            if not found:
                break
            time.sleep(0.5)

    # -- using it ---------------------------------------------------------
    def hypr(self, *args):
        return run(["hyprctl", *args], env=self.env)

    def clients(self):
        r = self.hypr("clients", "-j")
        return json.loads(r.stdout) if r.returncode == 0 and r.stdout.strip() else []

    def client(self, cls):
        return next((c for c in self.clients() if c["class"] == cls), None)

    def active_class(self):
        r = self.hypr("activewindow", "-j")
        with contextlib.suppress(ValueError):
            return json.loads(r.stdout).get("class")
        return None

    def spawn(self, cmd, env=None, name="proc"):
        e = dict(self.env, **(env or {}))
        out = open(os.path.join(self.dir, name + ".log"), "a")
        p = subprocess.Popen(cmd, env=e, stdout=out, stderr=subprocess.STDOUT, start_new_session=True)
        self.children.append(p)
        return p

    def run(self, cmd, env=None, timeout=20):
        return run(cmd, env=dict(self.env, **(env or {})), timeout=timeout)

    def log(self, name):
        with contextlib.suppress(OSError):
            return open(os.path.join(self.dir, name + ".log")).read()
        return ""

    def focus(self, cls):
        """Gives the keyboard to the window of class `cls`, as a click on it would."""
        window = self.client(cls)
        if not window:
            return False
        self.hypr("dispatch", 'hl.dsp.focus({ window = "address:%s" })' % window["address"])
        return bool(wait_for(3, lambda: self.active_class() == cls))

    def press(self, cls, key):
        """Sends one key to the window of class `cls` and to no other, through the compositor.
        It is refused when no such window is open, so a key can never land elsewhere."""
        window = self.client(cls)
        if not window:
            return False
        r = self.hypr(
            "dispatch",
            'hl.dsp.send_shortcut({ mods = "", key = "%s", window = "address:%s" })' % (key, window["address"]),
        )
        return r.stdout.strip() == "ok"

    def click(self, cls, dx, dy):
        """Left-clicks at (dx, dy) inside the window of class `cls`, and nowhere else. The pointer
        moves in steps first, as a real one does, so the toolkit sees hover before the press."""
        window = self.client(cls)
        if not window:
            return False
        x, y = window["at"][0] + dx, window["at"][1] + dy
        for ox, oy in ((-40, -25), (-12, -8), (-3, -2), (0, 0)):
            self.hypr("dispatch", "hl.dsp.cursor.move({ x = %d, y = %d })" % (x + ox, y + oy))
            time.sleep(0.08)
        time.sleep(0.25)
        r = self.hypr(
            "dispatch",
            'hl.dsp.send_shortcut({ mods = "", key = "mouse:272", window = "address:%s" })' % window["address"],
        )
        time.sleep(0.3)
        return r.stdout.strip() == "ok"

    def type_text(self, expect_class, text=None, key=None):
        """Type only when the window with the keyboard is the one expected; else refuse."""
        if wait_for(3, lambda: self.active_class() == expect_class) is None:
            return False
        if text:
            self.run(["wtype", text])
        if key:
            self.run(["wtype", "-k", key])
        return True
