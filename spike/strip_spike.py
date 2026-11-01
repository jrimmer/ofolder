#!/usr/bin/env python3
"""Spike: dock a chrome strip beside Flea's real save picker and try to steer it.

Not production. Answers: (1) can a strip sit docked to the picker and follow it,
(2) can it change the picker's directory by respawn, (3) by wtype keystrokes.
SPIKE_AUTO=1 runs the whole sequence unattended and screenshots each step.
"""
import json, os, signal, subprocess, sys, tempfile, time

import gi
gi.require_version("Gtk", "4.0")
from gi.repository import Gtk, GLib

HOME = os.path.expanduser("~")
PICKER_CLASS = "com.thisisgm.flea.picker"
STRIP_CLASS = "spike.strip"
STRIP_W = 230
GAP = 6
OUT = os.environ.get("SPIKE_OUT", tempfile.mkdtemp(prefix="spike-"))
LOG = open(os.path.join(OUT, "log.txt"), "a", buffering=1)


def log(msg):
    line = "%.2f %s" % (time.time() % 1000, msg)
    LOG.write(line + "\n")
    print(line, flush=True)


def hypr(*args):
    r = subprocess.run(["hyprctl", *args], capture_output=True, text=True)
    out = r.stdout.strip() + r.stderr.strip()
    log("hyprctl %s -> %s" % (" ".join(args), out))
    return out


def clients():
    try:
        return json.loads(subprocess.run(["hyprctl", "clients", "-j"], capture_output=True, text=True).stdout)
    except json.JSONDecodeError:
        return []


def find(cls, exclude=()):
    for c in clients():
        if c["class"] == cls and c["address"] not in exclude:
            return c
    return None


def descendants(pid):
    out, todo = [], [pid]
    while todo:
        p = todo.pop()
        kids = subprocess.run(["pgrep", "-P", str(p)], capture_output=True, text=True).stdout.split()
        out += [int(k) for k in kids]
        todo += [int(k) for k in kids]
    return out


def picker_folder(root_pid):
    """The folder the picker is showing: a directory fd held by its process tree (no IPC exists)."""
    for p in [root_pid] + descendants(root_pid):
        try:
            cwd = os.readlink("/proc/%d/cwd" % p)
            for fd in os.listdir("/proc/%d/fd" % p):
                t = os.readlink("/proc/%d/fd/%s" % (p, fd))
                if t.startswith("/") and os.path.isdir(t) and t != cwd:
                    return t
        except OSError:
            pass
    return None


class Spike:
    def __init__(self):
        self.child = None
        self.folder = HOME
        self.picker = None          # hyprctl client dict
        self.strip_win = None
        self.reply = os.path.join(OUT, "reply.json")
        self.stale = {c["address"] for c in clients() if c["class"] == PICKER_CLASS}

    # --- picker lifecycle (the contract read from /usr/lib/flea/flea-portal) ---
    def find_picker(self):
        """The window of the picker process we spawned (pid), never by reusable window address."""
        if not self.child:
            return None
        for c in clients():
            if c["class"] == PICKER_CLASS and c["pid"] == self.child.pid:
                return c
        return None

    def spawn_picker(self, folder, name="spike.txt"):
        self.folder = folder
        req = {"mode": "save", "title": "Spike save", "app": "spike", "accept": "",
               "multiple": False, "directory": False, "folder": folder, "file": "",
               "name": name, "files": [], "filters": [], "current": "", "currentIndex": -1}
        env = dict(os.environ, FLEA_PICKER=json.dumps(req))
        if os.path.exists(self.reply):
            os.remove(self.reply)
        self.child = subprocess.Popen(["flea", "--pick", self.reply], env=env)
        self.picker = None
        log("spawned picker pid=%d folder=%s" % (self.child.pid, folder))

    def kill_picker(self):
        if self.child and self.child.poll() is None:
            self.child.terminate()
            try:
                self.child.wait(2)
            except subprocess.TimeoutExpired:
                self.child.kill()
            log("killed picker")

    def respawn(self, folder):
        self.kill_picker()
        self.spawn_picker(folder)

    # --- docking ---
    def tick(self):
        if self.child and self.child.poll() is not None:
            log("picker exited rc=%s reply=%s" % (
                self.child.returncode,
                open(self.reply).read() if os.path.exists(self.reply) else "none"))
            self.child = None
            self.set_rule(False)
            if not os.environ.get("SPIKE_KEEP"):
                GLib.timeout_add(300, lambda: sys.exit(0))
            return True
        p = self.find_picker()
        if p:
            if self.picker is None:
                log("picker window found %s at=%s size=%s float=%s title=%r" % (
                    p["address"], p["at"], p["size"], p["floating"], p["title"]))
            self.picker = p
            if not self.presented:
                # Rule first, window second: the strip maps floating and in place, never tiled.
                self.presented = True
                self.set_rule(True, p)
                self.strip_win.present()
            self.dock(p)
        return True

    def set_rule(self, on, p=None):
        if on:
            x, y = p["at"][0] + p["size"][0] + GAP, p["at"][1]
            code = ('hl.window_rule({ name = "spike-strip", enabled = true, match = { class = "^%s$" }, '
                    'float = true, size = "%d %d", move = "%d %d" })' % (STRIP_CLASS.replace(".", "\\\\."), STRIP_W, p["size"][1], x, y))
        else:
            code = 'hl.window_rule({ name = "spike-strip", enabled = false })'
        log("rule %s -> %s" % ("on" if on else "off", hypr("eval", code)))

    def dock(self, p):
        s = find(STRIP_CLASS)
        if not s:
            return
        w = '"address:%s"' % s["address"]
        if not s["floating"]:
            hypr("dispatch", 'hl.dsp.window.float({ action = "on", window = %s })' % w)
        x, y = p["at"][0] + p["size"][0] + GAP, p["at"][1]
        want = ([x, y], [STRIP_W, p["size"][1]])
        if [s["at"], s["size"]] != [list(want[0]), list(want[1])]:
            hypr("dispatch", "hl.dsp.window.resize({ x = %d, y = %d, relative = false, window = %s })" % (STRIP_W, p["size"][1], w))
            hypr("dispatch", "hl.dsp.window.move({ x = %d, y = %d, relative = false, window = %s })" % (x, y, w))

    # --- steering via keystrokes (the unverified path) ---
    def wtype_goto(self, path):
        if not self.picker:
            return
        hypr("dispatch", 'hl.dsp.focus({ window = "address:%s" })' % self.picker["address"])
        time.sleep(0.3)
        subprocess.run(["wtype", "-M", "ctrl", "l", "-m", "ctrl"])
        time.sleep(0.3)
        subprocess.run(["wtype", path])
        time.sleep(0.2)
        subprocess.run(["wtype", "-k", "Return"])
        log("wtype goto %s" % path)

    def new_folder(self):
        base = (picker_folder(self.child.pid) if self.child else None) or self.folder
        path = os.path.join(base, "spike-folder-%d" % int(time.time()))
        os.makedirs(path)
        log("mkdir %s" % path)
        return path

    # --- UI ---
    def build(self, app):
        win = Gtk.ApplicationWindow(application=app, title="Spike strip")
        win.set_default_size(STRIP_W, 400)
        box = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=6, margin_top=10,
                      margin_bottom=10, margin_start=10, margin_end=10)
        win.set_child(box)
        box.append(Gtk.Label(label="Favorites (spike)", xalign=0))
        for label, path in (("Home", HOME), ("Work", HOME + "/Work"),
                            ("Documents", HOME + "/Documents"), ("Downloads", HOME + "/Downloads")):
            b = Gtk.Button(label="→ " + label)
            b.connect("clicked", lambda _b, p=path: self.respawn(p))
            box.append(b)
        box.append(Gtk.Separator())
        nf = Gtk.Button(label="+ New folder")
        nf.connect("clicked", lambda _b: self.wtype_goto(self.new_folder()))
        box.append(nf)
        wt = Gtk.Button(label="wtype → ~/Documents")
        wt.connect("clicked", lambda _b: GLib.timeout_add(50, lambda: self.wtype_goto(HOME + "/Documents") and False))
        box.append(wt)
        self.strip_win = win
        self.presented = False
        GLib.timeout_add(200, self.tick)
        self.spawn_picker(HOME)
        if os.environ.get("SPIKE_AUTO"):
            GLib.timeout_add(100, self.auto)

    # --- unattended run with screenshots ---
    def shot(self, name):
        p, s = self.find_picker(), find(STRIP_CLASS)
        if not (p and s):
            log("shot %s skipped: picker=%s strip=%s" % (name, bool(p), bool(s)))
            return
        x, y = min(p["at"][0], s["at"][0]), min(p["at"][1], s["at"][1])
        x2 = max(p["at"][0] + p["size"][0], s["at"][0] + s["size"][0])
        y2 = max(p["at"][1] + p["size"][1], s["at"][1] + s["size"][1])
        geom = "%d,%d %dx%d" % (x, y, x2 - x, y2 - y)
        out = os.path.join(OUT, name + ".png")
        subprocess.run(["grim", "-g", geom, out])
        log("shot %s geom=%s title=%r" % (name, geom, p["title"]))

    def auto(self):
        play = os.path.join(OUT, "playground")
        sub = os.path.join(play, "sub")
        os.makedirs(sub, exist_ok=True)
        pid = lambda: self.child.pid
        def check(label, want):
            got = picker_folder(pid())
            log("CHECK %-22s want=%s got=%s %s" % (label, want, got, "OK" if got == want else "MISMATCH"))
        def newfolder():
            p = self.new_folder()
            log("new_folder created %s (parent matches read folder: %s)" % (p, os.path.dirname(p) == picker_folder(pid())))
            self.wtype_goto(p)
        def drag():
            a = "address:" + self.picker["address"]
            hypr("dispatch", "hl.dsp.window.move({ x = 200, y = 100, relative = true, window = \"%s\" })" % a)
        def report_follow():
            p, s_ = self.find_picker(), find(STRIP_CLASS)
            ok = p and s_ and s_["at"] == [p["at"][0] + p["size"][0] + GAP, p["at"][1]]
            log("FOLLOW picker.at=%s strip.at=%s %s" % (p and p["at"], s_ and s_["at"], "OK" if ok else "MISMATCH"))
        steps = [
            (4.0, lambda: self.shot("1-docked")),
            (5.0, lambda: self.respawn(play)),
            (9.0, lambda: check("after respawn", play)),
            (10.0, lambda: self.wtype_goto(sub)),
            (13.0, lambda: check("after in-picker nav", sub)),
            (14.0, newfolder),
            (17.5, lambda: self.shot("2-new-folder-entered")),
            (18.0, lambda: log("final folder read: %s" % picker_folder(pid()))),
            (19.0, drag),
            (20.5, report_follow),
            (21.0, lambda: self.shot("3-after-drag")),
            (22.0, lambda: (subprocess.run(["wtype", "-k", "Escape"]), log("sent Escape"))),
        ]
        for delay, fn in steps:
            GLib.timeout_add(int(delay * 1000), lambda f=fn: (f(), False)[1])
        return False


def main():
    log("out dir " + OUT)
    spike = Spike()
    app = Gtk.Application(application_id=STRIP_CLASS)
    app.connect("activate", spike.build)
    signal.signal(signal.SIGINT, lambda *_: (spike.kill_picker(), sys.exit(0)))
    app.run([])
    spike.kill_picker()


if __name__ == "__main__":
    main()
