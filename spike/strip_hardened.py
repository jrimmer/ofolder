#!/usr/bin/env python3
"""Hardened spike: chrome strip docked to Flea's real picker.

Differences from strip_spike.py (the baseline):
  - never types unless the picker is verified to be the active window (wtype goes to the focused window)
  - folder reading is stable-or-nothing; "+ New folder" refuses rather than guessing, and never collides
  - dock flips to the left at the monitor edge, follows resize and workspace changes
  - clean shutdown on SIGTERM/SIGINT/picker exit: picker killed, rule disabled
SPIKE_SUITE=guard|layout|paths runs unattended checks and logs PASS/FAIL lines; SPIKE_LIVE=1 is for a human.
"""
import atexit, ctypes, json, os, signal, subprocess, sys, tempfile, time

import gi
gi.require_version("Gtk", "4.0")
from gi.repository import Gtk, GLib, Pango

HOME = os.path.expanduser("~")
RUNTIME = os.environ.get("XDG_RUNTIME_DIR", "/run/user/%d" % os.getuid())
PICKER_CLASS = "com.thisisgm.flea.picker"
STRIP_CLASS = "spike.strip"
STRIP_W, GAP = 230, 6
OUT = os.environ.get("SPIKE_OUT", tempfile.mkdtemp(prefix="spike-"))
LOG = open(os.path.join(OUT, "log.txt"), "a", buffering=1)
# Flea's own state dirs: never the folder the user is looking at.
FLEA_STATE = tuple(os.path.join(HOME, p) for p in (".cache", ".local/state", ".local/share/flea", ".config/flea"))


def log(msg):
    LOG.write("%.2f %s\n" % (time.time() % 1000, msg))


def hypr(*args):
    r = subprocess.run(["hyprctl", *args], capture_output=True, text=True)
    out = (r.stdout + r.stderr).strip()
    if out != "ok":
        log("hyprctl %s -> %s" % (" ".join(args)[:150], out[:150]))
    return out


def hjson(*args):
    try:
        return json.loads(subprocess.run(["hyprctl", *args, "-j"], capture_output=True, text=True).stdout)
    except json.JSONDecodeError:
        return None


def find_class(cls):
    return next((c for c in hjson("clients") or [] if c["class"] == cls), None)


def active_address():
    return (hjson("activewindow") or {}).get("address")


def mon_bounds(client):
    for m in hjson("monitors") or []:
        if m["id"] == client["monitor"]:
            sc = m.get("scale") or 1
            rl, rt, rr, rb = m.get("reserved", [0, 0, 0, 0])
            return (m["x"] + rl, m["y"] + rt, m["x"] + m["width"] / sc - rr, m["y"] + m["height"] / sc - rb)
    return None


def strip_geom(p):
    """Where the strip belongs for picker client p: right edge, else left, else overlap; clamped to the monitor."""
    b = mon_bounds(p)
    w, h = STRIP_W, p["size"][1]
    x, side = p["at"][0] + p["size"][0] + GAP, "right"
    y = p["at"][1]
    if b:
        if x + w > b[2]:
            xl = p["at"][0] - GAP - w
            x, side = (xl, "left") if xl >= b[0] else (b[2] - w, "overlap")
        h = min(h, b[3] - b[1])
        y = max(b[1], min(y, b[3] - h))
    return int(x), int(y), w, int(h), side


# --- reading the picker's folder (no IPC exists: the backend helper holds a directory fd) ---
def descendants(pid):
    out, todo = [], [pid]
    while todo:
        p = todo.pop()
        kids = subprocess.run(["pgrep", "-P", str(p)], capture_output=True, text=True).stdout.split()
        out += [int(k) for k in kids]
        todo += [int(k) for k in kids]
    return out


def dir_fds(root_pid, backend_only=True):
    found = []
    for p in [root_pid] + descendants(root_pid):
        try:
            cmd = open("/proc/%d/cmdline" % p).read()
            if backend_only and "--backend" not in cmd:
                continue
            cwd = os.readlink("/proc/%d/cwd" % p)
            for fd in os.listdir("/proc/%d/fd" % p):
                t = os.readlink("/proc/%d/fd/%s" % (p, fd))
                if t.startswith("/") and os.path.isdir(t) and t != cwd:
                    found.append(t)
        except OSError:
            pass
    return found


def candidates(root_pid):
    return {t for t in dir_fds(root_pid)
            if not t.startswith(FLEA_STATE) and not t.startswith(RUNTIME)}


def one_folder(root_pid):
    c = candidates(root_pid)
    return next(iter(c)) if len(c) == 1 else None


def stable_folder(root_pid, tries=8, gap=0.03):
    """Two consecutive identical single-candidate readings, or None. Never a guess."""
    last = None
    for _ in range(tries):
        cur = one_folder(root_pid)
        if cur is not None and cur == last:
            return cur
        last = cur
        time.sleep(gap)
    return None


def die_with_parent():
    """Child gets SIGTERM if the strip dies for any reason, even SIGKILL (PR_SET_PDEATHSIG = 1)."""
    ctypes.CDLL("libc.so.6", use_errno=True).prctl(1, signal.SIGTERM)


class Spike:
    def __init__(self):
        self.child = None
        self.picker = None
        self.presented = False
        self.shut = False
        self.app = None
        self.last_seen_folder = None
        self.reply = os.path.join(OUT, "reply.json")

    # --- picker lifecycle ---
    def alive(self):
        return self.child is not None and self.child.poll() is None

    def find_picker(self):
        """Window of the process we spawned, by pid: window addresses are reused by the compositor."""
        if not self.child:
            return None
        return next((c for c in hjson("clients") or [] if c["class"] == PICKER_CLASS and c["pid"] == self.child.pid), None)

    def spawn_picker(self, folder, name="spike.txt"):
        req = {"mode": "save", "title": "Spike save", "app": "spike", "accept": "", "multiple": False,
               "directory": False, "folder": folder, "file": "", "name": name, "files": [], "filters": [],
               "current": "", "currentIndex": -1}
        if os.path.exists(self.reply):
            os.remove(self.reply)
        self.child = subprocess.Popen(["flea", "--pick", self.reply], env=dict(os.environ, FLEA_PICKER=json.dumps(req)),
                                      stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
                                      preexec_fn=die_with_parent)
        self.picker = None
        log("spawned picker pid=%d folder=%s" % (self.child.pid, folder))

    def kill_picker(self):
        if self.alive():
            self.child.terminate()
            try:
                self.child.wait(2)
            except subprocess.TimeoutExpired:
                self.child.kill()
            log("killed picker")

    def respawn(self, folder):
        self.kill_picker()
        self.spawn_picker(folder)

    def shutdown(self):
        if self.shut:
            return
        self.shut = True
        self.kill_picker()
        self.set_rule(False)

    # --- window rule: strip maps floating and in place ---
    def set_rule(self, on, p=None):
        if on:
            x, y, w, h, _ = strip_geom(p)
            code = ('hl.window_rule({ name = "spike-strip", enabled = true, match = { class = "^spike\\\\.strip$" }, '
                    'float = true, size = "%d %d", move = "%d %d" })' % (w, h, x, y))
        else:
            code = 'hl.window_rule({ name = "spike-strip", enabled = false })'
        log("rule %s -> %s" % ("on" if on else "off", hypr("eval", code)))

    # --- periodic: follow, flip, workspace, folder readout ---
    def tick(self):
        if self.shut:
            return False
        if self.child and self.child.poll() is not None:
            log("picker exited rc=%s reply=%s" % (self.child.returncode,
                open(self.reply).read() if os.path.exists(self.reply) else "none"))
            self.child = None
            self.shutdown()
            GLib.timeout_add(200, lambda: (self.app.quit(), False)[1])
            return False
        p = self.find_picker()
        if p:
            if self.picker is None:
                log("picker window found at=%s size=%s" % (p["at"], p["size"]))
            self.picker = p
            if not self.presented:
                self.presented = True
                self.set_rule(True, p)
                self.win.present()
            self.dock(p)
            cur = one_folder(self.child.pid)
            self.now_label.set_text("Now: " + (cur or "…"))
            if cur and cur != self.last_seen_folder:
                log("FOLDER -> %s" % cur)
                self.last_seen_folder = cur
        return True

    def dock(self, p):
        s = find_class(STRIP_CLASS)
        if not s:
            return
        w = '"address:%s"' % s["address"]
        if not s["floating"]:
            hypr("dispatch", 'hl.dsp.window.float({ action = "on", window = %s })' % w)
        if s["workspace"]["id"] != p["workspace"]["id"]:
            hypr("dispatch", 'hl.dsp.window.move({ workspace = "%s", follow = false, window = %s })' % (p["workspace"]["name"], w))
        x, y, sw, sh, side = strip_geom(p)
        if abs(s["at"][0] - x) > 1 or abs(s["at"][1] - y) > 1 or abs(s["size"][0] - sw) > 1 or abs(s["size"][1] - sh) > 1:
            hypr("dispatch", "hl.dsp.window.resize({ x = %d, y = %d, relative = false, window = %s })" % (sw, sh, w))
            hypr("dispatch", "hl.dsp.window.move({ x = %d, y = %d, relative = false, window = %s })" % (x, y, w))

    # --- keystrokes: only ever into the verified-active picker ---
    def focus_picker(self, dispatch=True):
        p = self.find_picker()
        if not p:
            return False
        if dispatch:
            hypr("dispatch", 'hl.dsp.focus({ window = "address:%s" })' % p["address"])
        for _ in range(8):
            if active_address() == p["address"]:
                return True
            time.sleep(0.08)
        return False

    def safe_wtype(self, *args):
        p = self.find_picker()
        if not p or active_address() != p["address"]:
            log("REFUSED to type: active window is not the picker (active=%s)" % active_address())
            return False
        subprocess.run(["wtype", *args])
        return True

    def goto(self, path, dispatch=True):
        """Navigate the picker in place (keeps its filename and selection). False if refused."""
        if not os.path.isabs(path) or any(ord(c) < 32 or ord(c) == 127 for c in path) or len(path) > 4000:
            log("REFUSED path (not absolute, control chars, or too long): %r" % path[:80])
            return False
        if not self.alive() or not self.focus_picker(dispatch):
            log("REFUSED goto %r: picker not alive or not focusable" % path[:80])
            return False
        return self.type_raw(path, dispatch=False)

    def type_raw(self, text, dispatch=True):
        """Ctrl+L, text, Return into the verified-active picker."""
        if dispatch and not self.focus_picker():
            log("REFUSED type_raw %r: picker not focusable" % text[:60])
            return False
        ok = (self.safe_wtype("-M", "ctrl", "l", "-m", "ctrl") and (time.sleep(0.25) or True)
              and self.safe_wtype("--", text) and (time.sleep(0.15) or True)
              and self.safe_wtype("-k", "Return"))
        log("typed %s -> %s" % (text[:100], "sent" if ok else "ABORTED"))
        return ok

    def navigate(self, path):
        """Go there and verify the picker arrived; otherwise respawn into it. Returns how it got there."""
        if self.goto(path) and self.wait_folder({path, os.path.realpath(path)}, 1.5) is not None:
            return "in-place"
        log("fallback: respawn into %s" % path)
        if self.alive():
            self.respawn(path)
            return "respawn"
        return "none"

    def make_folder(self):
        """mkdir in the picker's real current folder; refuses when the folder is not known for certain."""
        base = stable_folder(self.child.pid) if self.alive() else None
        if not base:
            log("REFUSED new folder: picker folder unknown or ambiguous: %s" % sorted(candidates(self.child.pid)) if self.alive() else "REFUSED new folder: picker gone")
            return None
        for i in range(1, 200):
            path = os.path.join(base, "New folder" if i == 1 else "New folder %d" % i)
            try:
                os.mkdir(path)
                log("mkdir %s" % path)
                return path
            except FileExistsError:
                continue
            except OSError as e:
                log("FAILED mkdir in %s: %s" % (base, e))
                return None
        return None

    def new_folder_clicked(self):
        path = self.make_folder()
        if path:
            self.navigate(path)

    # --- UI ---
    def build(self, app):
        self.app = app
        self.win = Gtk.ApplicationWindow(application=app, title="Spike strip")
        self.win.set_default_size(STRIP_W, 400)
        box = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=6, margin_top=10, margin_bottom=10,
                      margin_start=10, margin_end=10)
        self.win.set_child(box)
        self.now_label = Gtk.Label(label="Now: …", xalign=0, ellipsize=Pango.EllipsizeMode.START, max_width_chars=26)
        box.append(self.now_label)
        box.append(Gtk.Separator())
        for label, path in (("Home", HOME), ("Work", HOME + "/Work"), ("Documents", HOME + "/Documents"),
                            ("Downloads", HOME + "/Downloads")):
            b = Gtk.Button(label="→ " + label)
            b.connect("clicked", lambda _b, p=path: self.navigate(p))
            box.append(b)
        box.append(Gtk.Separator())
        nf = Gtk.Button(label="+ New folder")
        nf.connect("clicked", lambda _b: self.new_folder_clicked())
        box.append(nf)
        GLib.timeout_add(200, self.tick)
        start = os.environ.get("SPIKE_START", HOME)
        self.spawn_picker(start)
        suite = os.environ.get("SPIKE_SUITE")
        if suite:
            self.run_suite(getattr(self, "suite_" + suite))

    def run_suite(self, gen_fn):
        gen = gen_fn()

        def step():
            try:
                delay = next(gen)
            except StopIteration:
                return False
            except Exception as e:  # a crash in a test is a test failure, loudly
                log("SUITE CRASH: %r" % e)
                return False
            GLib.timeout_add(int(delay * 1000), step)
            return False
        step()

    # --- assertions / evidence ---
    def check(self, name, ok, detail=""):
        log("%s %s %s" % ("PASS" if ok else "FAIL", name, detail))

    def shot(self, name):
        p, s = self.find_picker(), find_class(STRIP_CLASS)
        if not (p and s):
            return log("shot %s skipped" % name)
        x, y = min(p["at"][0], s["at"][0]), min(p["at"][1], s["at"][1])
        x2 = max(p["at"][0] + p["size"][0], s["at"][0] + s["size"][0])
        y2 = max(p["at"][1] + p["size"][1], s["at"][1] + s["size"][1])
        subprocess.run(["grim", "-g", "%d,%d %dx%d" % (x, y, x2 - x, y2 - y), os.path.join(OUT, name + ".png")])
        log("shot %s" % name)

    def wait_folder(self, want, timeout=4.0):
        t0 = time.time()
        while time.time() - t0 < timeout:
            if one_folder(self.child.pid) in want:
                return time.time() - t0
            time.sleep(0.015)
        return None

    def finish(self):
        for _ in range(3):
            if not self.alive():
                return
            if self.focus_picker():
                self.safe_wtype("-k", "Escape")
            time.sleep(0.8)
        self.kill_picker()

    # ===== suites =====
    def suite_guard(self):
        play = os.path.join(OUT, "playground")
        os.makedirs(play + "/target", exist_ok=True)
        yield 4.5
        self.respawn(play)
        yield 3.5
        before = stable_folder(self.child.pid)
        # Put focus on the STRIP and ask to type without re-focusing the picker: it must refuse.
        s = find_class(STRIP_CLASS)
        hypr("dispatch", 'hl.dsp.focus({ window = "address:%s" })' % s["address"])
        time.sleep(0.4)
        sent = self.goto(play + "/target", dispatch=False)
        time.sleep(1.0)
        after = stable_folder(self.child.pid)
        self.check("guard refuses when picker not focused", sent is False and before == after, "sent=%s before=%s after=%s" % (sent, before == play, after == play))
        # New folder: refuses when the reading is ambiguous (simulated), collision-safe otherwise.
        a, b = self.make_folder(), self.make_folder()
        self.check("new folder names never collide", a and b and a != b and os.path.basename(b) == "New folder 2", "%s %s" % (a and os.path.basename(a), b and os.path.basename(b)))
        real, fake = candidates, None
        globals()["candidates"] = lambda pid: {"/tmp", "/var"}
        refused = self.make_folder()
        globals()["candidates"] = real
        self.check("new folder refuses on ambiguous folder", refused is None)
        # Unwritable folder: handled, not a crash.
        self.goto("/usr")
        self.wait_folder({"/usr"})
        r = self.make_folder()
        self.check("new folder in unwritable dir fails cleanly", r is None and self.alive())
        # After the picker is gone, buttons must be inert.
        self.kill_picker()
        before = set(os.listdir(play))
        self.check("make_folder and goto inert after picker exit",
                   self.make_folder() is None and self.goto(play) is False and set(os.listdir(play)) == before)
        yield 1.0

    def suite_layout(self):
        yield 4.5
        p0 = self.find_picker()
        pid = lambda: self.find_picker()
        addr = '"address:%s"' % p0["address"]
        b = mon_bounds(p0)
        # Flip: push the picker against the monitor's right edge.
        hypr("dispatch", "hl.dsp.window.move({ x = %d, y = %d, relative = false, window = %s })" % (b[2] - p0["size"][0] - 10, p0["at"][1], addr))
        yield 1.2
        p, s = pid(), find_class(STRIP_CLASS)
        self.check("strip flips left near right monitor edge", s["at"][0] + s["size"][0] <= p["at"][0] and s["at"][0] >= b[0],
                   "picker.x=%s strip=%s..%s monitor.right=%s" % (p["at"][0], s["at"][0], s["at"][0] + s["size"][0], b[2]))
        self.shot("flip-left")
        # Back to the middle: strip returns to the right edge.
        hypr("dispatch", "hl.dsp.window.move({ x = %d, y = %d, relative = false, window = %s })" % (p0["at"][0], p0["at"][1], addr))
        yield 1.2
        p, s = pid(), find_class(STRIP_CLASS)
        self.check("strip returns to right edge", s["at"][0] == p["at"][0] + p["size"][0] + GAP, "strip.x=%s want=%s" % (s["at"][0], p["at"][0] + p["size"][0] + GAP))
        # Resize follows.
        hypr("dispatch", "hl.dsp.window.resize({ x = 700, y = 480, relative = false, window = %s })" % addr)
        yield 1.2
        p, s = pid(), find_class(STRIP_CLASS)
        self.check("strip height follows picker resize", s["size"][1] == p["size"][1] and s["at"][1] == p["at"][1] and s["at"][0] == p["at"][0] + p["size"][0] + GAP,
                   "picker=%s strip=%s" % (p["size"], s["size"]))
        hypr("dispatch", "hl.dsp.window.resize({ x = %d, y = %d, relative = false, window = %s })" % (p0["size"][0], p0["size"][1], addr))
        yield 1.0
        # Workspace follow (follow=false: your view does not change).
        ws0 = p0["workspace"]["id"]
        target = 9 if ws0 != 9 else 8
        hypr("dispatch", 'hl.dsp.window.move({ workspace = "%d", follow = false, window = %s })' % (target, addr))
        yield 1.5
        p, s = pid(), find_class(STRIP_CLASS)
        self.check("strip follows picker to another workspace", s["workspace"]["id"] == p["workspace"]["id"] == target,
                   "picker ws=%s strip ws=%s" % (p["workspace"]["id"], s["workspace"]["id"]))
        hypr("dispatch", 'hl.dsp.window.move({ workspace = "%d", follow = false, window = %s })' % (ws0, addr))
        yield 1.5
        p, s = pid(), find_class(STRIP_CLASS)
        self.check("strip follows picker back", s["workspace"]["id"] == p["workspace"]["id"] == ws0, "picker ws=%s strip ws=%s" % (p["workspace"]["id"], s["workspace"]["id"]))
        self.shot("layout-final")
        self.finish()
        yield 1.5

    def suite_paths(self):
        play = os.path.join(OUT, "playground")
        os.makedirs(play, exist_ok=True)
        names = {
            "space": "with space", "unicode": "ünïcödé 日本語", "quotes": "quo'te\"d", "dash": "-dash",
            "hidden": ".hidden", "long": "x" * 200, "deep": "/".join("d%d" % i for i in range(15)),
            "shellmeta": "$(touch PWNED);`id`;|&", "empty": "empty", "big": "big",
        }
        for rel in names.values():
            os.makedirs(os.path.join(play, rel), exist_ok=True)
        for i in range(5000):
            open(os.path.join(play, "big", "f%d" % i), "w").close()
        os.symlink(os.path.join(play, "with space"), os.path.join(play, "link-to-space"))
        names["symlink"] = "link-to-space"
        yield 4.5
        self.respawn(play)
        yield 3.5
        lat = []
        for label, rel in names.items():
            path = os.path.join(play, rel)
            if not self.goto(path):
                self.check("path:" + label, False, "goto refused")
                continue
            want = {path, os.path.realpath(path)}
            t = self.wait_folder(want)
            got = one_folder(self.child.pid)
            self.check("path:" + label, t is not None, "latency=%s reported=%s" % ("%.0fms" % (t * 1000) if t else "NONE", got if got not in want else ("(realpath)" if got != path else "(as typed)")))
            if t:
                lat.append(t)
            yield 0.2
        self.check("shell metacharacters not executed", not os.path.exists(os.path.join(play, "PWNED")) and not os.path.exists(os.path.join(os.getcwd(), "PWNED")))
        if lat:
            log("INFO latency min=%.0fms max=%.0fms n=%d" % (min(lat) * 1000, max(lat) * 1000, len(lat)))
        # Informational probes: what does the reading look like in odd places?
        for label, target in (("unreadable", "/root"), ("missing", "/nonexistent-xyz"), ("trash", "trash:///"),
                              ("recent", "recent:///"), ("images-thumbnails", HOME + "/Pictures/Screenshots")):
            if target.startswith("/"):
                self.goto(target)
            else:  # not absolute: typed by hand, still guarded
                if self.focus_picker():
                    self.safe_wtype("-M", "ctrl", "l", "-m", "ctrl"); time.sleep(0.25)
                    self.safe_wtype("--", target); time.sleep(0.15); self.safe_wtype("-k", "Return")
            yield 2.0
            log("PROBE %s -> one_folder=%s stable=%s all_dir_fds=%s picker_alive=%s" % (
                label, one_folder(self.child.pid), stable_folder(self.child.pid), sorted(dir_fds(self.child.pid, backend_only=False))[:4], self.find_picker() is not None))
        self.finish()
        yield 1.5

    def suite_rootprobe(self):
        def tree(tag):
            kids = [(p, open("/proc/%d/cmdline" % p).read().replace("\0", " ")[:40]) for p in descendants(self.child.pid) if os.path.exists("/proc/%d" % p)]
            log("TREE %s: one_folder=%s kids=%s" % (tag, one_folder(self.child.pid), kids))
        yield 4.5
        tree("start"); self.shot("p0-start")
        self.goto("/root"); yield 2.5
        tree("after /root"); self.shot("p1-root")
        self.goto(HOME + "/Documents"); yield 2.5
        tree("after Documents"); self.shot("p2-documents")
        self.respawn(HOME + "/Documents"); yield 4.0
        tree("after respawn"); self.shot("p3-respawn")
        self.finish(); yield 1.5

    def suite_recover(self):
        docs = HOME + "/Documents"
        yield 4.5
        for label, bad in (("missing", "/nonexistent-xyz"), ("trash", "trash:///"), ("recent", "recent:///"),
                           ("relative", "no/such/rel"), ("tilde", "~"), ("unreadable", "/root")):
            self.type_raw(bad)
            yield 1.5
            self.shot("r-" + label)
            before = one_folder(self.child.pid)
            how = self.navigate(docs)
            yield 2.0 if how == "respawn" else 0.3
            got = stable_folder(self.child.pid)
            self.check("recover after %s" % label, got == docs, "bad-state-reading=%s via=%s got=%s" % (before, how, got))
        self.finish(); yield 1.5

    def suite_stress(self):
        a, b = HOME + "/Documents", HOME + "/Work"
        yield 4.5
        bad = fallbacks = 0
        garbles = []
        for i in range(40):
            want = a if i % 2 == 0 else b
            self.goto(want)
            t = self.wait_folder({want}, 1.5)
            if t is None:
                bad += 1
                # what did the path bar end up with? the picker reading + a screenshot is the evidence
                self.shot("stress-bad-%d" % i)
                log("STRESS miss #%d want=%s reading=%s" % (i, want, one_folder(self.child.pid)))
                how = self.navigate(want)
                fallbacks += 1
                yield 2.0 if how == "respawn" else 0.3
            else:
                yield 0.1
        self.check("stress 40 navigations", bad == 0, "misses=%d fallbacks=%d" % (bad, fallbacks))
        self.finish(); yield 1.5


def main():
    log("out dir " + OUT)
    spike = Spike()
    app = Gtk.Application(application_id=STRIP_CLASS)
    app.connect("activate", spike.build)
    quit_ = lambda: (spike.shutdown(), app.quit(), False)[2]
    GLib.unix_signal_add(GLib.PRIORITY_DEFAULT, signal.SIGTERM, quit_)
    GLib.unix_signal_add(GLib.PRIORITY_DEFAULT, signal.SIGINT, quit_)
    atexit.register(spike.shutdown)
    app.run([])
    spike.shutdown()


if __name__ == "__main__":
    main()
