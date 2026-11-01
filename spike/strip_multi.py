#!/usr/bin/env python3
"""ofolder spike: ONE strip docked beside Flea's, GTK's or Strata's real save dialog, behind a common adapter interface.

usage: SPIKE_ADAPTER=flea|gtk|strata [SPIKE_AUTO=1] [SPIKE_START=/folder] strip_multi.py
All backend-specific knowledge lives in the adapters (see docs/adapters.md); the strip never touches a backend directly.
"""
import atexit, os, signal, subprocess, sys, time

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
os.environ.setdefault("SPIKE_OUT", os.path.join(HERE, "runs", "multi-%s" % os.environ.get("SPIKE_ADAPTER", "flea")))
os.makedirs(os.environ["SPIKE_OUT"], exist_ok=True)
import strip_hardened as sh          # reuse: logging, hyprctl helpers, geometry, Flea mechanics
from strip_hardened import log, hypr, hjson, find_class, strip_geom, STRIP_CLASS, STRIP_W, GAP, HOME, OUT

import gi
gi.require_version("Gtk", "4.0")
gi.require_version("Atspi", "2.0")
from gi.repository import Gtk, GLib, Pango, Atspi
Atspi.init()


# ---------------------------------------------------------------- accessibility helpers (GTK + Strata)
def a11y_app(name):
    d = Atspi.get_desktop(0)
    return next((d.get_child_at_index(i) for i in range(d.get_child_count()) if name in (d.get_child_at_index(i).get_name() or "")), None)


def dfs(n, out=None):
    out = [] if out is None else out
    out.append(n)
    for j in range(n.get_child_count()):
        dfs(n.get_child_at_index(j), out)
    return out


def find_node(root, role, name=None):
    return next((n for n in dfs(root) if n.get_role_name() == role and (name is None or n.get_name() == name)), None)


def press(node):
    return bool(node) and bool(Atspi.Action.do_action(node, 0))


def active_is(address):
    return (hjson("activewindow") or {}).get("address") == address


# ---------------------------------------------------------------- adapters
class Adapter:
    name = "?"
    needs_float = False           # dialog opens tiled: install a float rule before launch and centre it
    def launch(self, folder): raise NotImplementedError
    def window(self): raise NotImplementedError         # hyprctl client of OUR dialog, or None
    def read_folder(self): raise NotImplementedError    # folder shown, or None. Never a guess
    def goto(self, path): raise NotImplementedError     # True if the request was issued; caller verifies arrival
    def close(self): raise NotImplementedError
    def alive(self): raise NotImplementedError
    def kill(self): pass


class FleaAdapter(Adapter):
    name = "flea"
    def launch(self, folder):
        self.s = sh.Spike()
        self.s.spawn_picker(folder)
    def window(self): return self.s.find_picker()
    def read_folder(self): return sh.one_folder(self.s.child.pid) if self.s.alive() else None
    def goto(self, path): return self.s.goto(path)
    def close(self):
        for _ in range(3):
            if not self.s.alive(): return
            if self.s.focus_picker(): self.s.safe_wtype("-k", "Escape")
            time.sleep(0.6)
        self.s.kill_picker()
    def alive(self): return self.s.alive()
    def kill(self): self.s.kill_picker()


class PortalCallAdapter(Adapter):
    """Backends that live in a resident daemon: we call their D-Bus impl directly and match our window by title."""
    backend = ""
    title = ""
    window_class = ""
    def launch(self, folder):
        self.call = subprocess.Popen([sys.executable, os.path.join(HERE, "portal_call.py"), self.backend, folder, self.title],
                                     stdout=open(os.path.join(OUT, "call.out"), "w"), stderr=subprocess.STDOUT)
        log("%s: dialog requested (call pid %d)" % (self.name, self.call.pid))
    def window(self):
        return next((c for c in hjson("clients") or [] if c["class"] == self.window_class and c["title"] == self.title), None)
    def alive(self): return self.call.poll() is None
    def kill(self):
        if self.call.poll() is None:
            self.close()
            try: self.call.wait(3)
            except subprocess.TimeoutExpired: self.call.kill()


class GtkAdapter(PortalCallAdapter):
    name, backend, title, window_class = "gtk", "gtk", "ofolder GTK dialog", "xdg-desktop-portal-gtk"
    def _chooser(self):
        app = a11y_app("xdg-desktop-portal-gtk")
        return next((app.get_child_at_index(i) for i in range(app.get_child_count())
                     if app.get_child_at_index(i).get_role_name() == "file chooser" and app.get_child_at_index(i).get_name() == self.title), None) if app else None
    def read_folder(self):
        """Path bar toggle buttons, deepest first; the CHECKED one is the current folder."""
        try:
            pb = find_node(self._chooser(), "filler", "PathBar Layer")
            row = pb.get_child_at_index(0)
            btns = [row.get_child_at_index(j) for j in range(row.get_child_count())]
            btns = [b for b in btns if b.get_role_name() == "toggle button"]
            names = [b.get_name() for b in btns]
            ck = [i for i, b in enumerate(btns) if b.get_state_set().contains(Atspi.StateType.CHECKED)]
            if len(ck) != 1 or "File System Root" not in names[ck[0]:]:
                return None
            path = "/" + "/".join(reversed(names[ck[0]:names.index("File System Root", ck[0])]))
            return path if os.path.isdir(path) else None
        except Exception:
            return None
    def goto(self, path):
        """No keystrokes: set the Name entry to 'path/' and fire its activate action twice (the first is swallowed after a move)."""
        try:
            ent = self._entry()
            if getattr(self, "_saved_name", None) is None:      # the app's suggested filename: GTK clears the entry on arrival
                self._saved_name = Atspi.Text.get_text(ent, 0, Atspi.Text.get_character_count(ent))
            Atspi.EditableText.set_text_contents(ent, path.rstrip("/") + "/")
            Atspi.Action.do_action(ent, 0)
            time.sleep(0.4)
            Atspi.Action.do_action(ent, 0)
            return True
        except Exception as e:
            log("gtk goto failed: %r" % e)
            return False
    def _entry(self):
        return next(t for t in dfs(self._chooser()) if t.get_role_name() == "text" and t.get_state_set().contains(Atspi.StateType.EDITABLE))
    def after_goto(self):
        """Put the app's filename back once the folder change has landed."""
        saved, self._saved_name = getattr(self, "_saved_name", None), None
        if saved is not None:
            try: Atspi.EditableText.set_text_contents(self._entry(), saved)
            except Exception as e: log("gtk restore name failed: %r" % e)
    def read_name(self):
        try:
            e = self._entry()
            return Atspi.Text.get_text(e, 0, Atspi.Text.get_character_count(e))
        except Exception:
            return None
    def close(self):
        try:
            press(find_node(self._chooser(), "button", "Cancel"))
        except Exception:
            pass


class StrataAdapter(PortalCallAdapter):
    name, backend, title, window_class = "strata", "strata", "ofolder Strata dialog", "io.github.lgse.Strata.FileChooser"
    needs_float = True
    PLACES = {HOME: "Home", HOME + "/Documents": "Documents", HOME + "/Downloads": "Downloads", HOME + "/Music": "Music",
              HOME + "/Pictures": "Pictures", HOME + "/Videos": "Videos", HOME + "/Projects": "Projects"}
    def _root(self):
        return a11y_app("Strata.FileChooser")
    def read_folder(self):
        """Join the breadcrumb labels that share a container with the 'Copy path' button: ~ / Work => ~/Work."""
        try:
            cp = find_node(self._root(), "button", "Copy path")
            anc = cp.get_parent()
            for _ in range(4):
                t = "".join(n.get_name() for n in dfs(anc) if n.get_role_name() == "label")
                if t.startswith("~") or t.startswith("/"):
                    p = HOME + t[1:] if t.startswith("~") else t
                    return p if os.path.isdir(p) else None
                anc = anc.get_parent()
        except Exception:
            return None
    def goto(self, path):
        place = self.PLACES.get(path.rstrip("/") or "/")
        if place:                                         # no keystrokes: click the sidebar place
            return press(find_node(self._root(), "button", place))
        w = self.window()                                 # arbitrary path: Ctrl+L, only into the verified-active dialog
        if not w or any(ord(c) < 32 for c in path):
            return False
        hypr("dispatch", 'hl.dsp.focus({ window = "address:%s" })' % w["address"])
        for step in (["-M", "ctrl", "l", "-m", "ctrl"], ["--", path], ["-k", "Return"]):
            time.sleep(0.25)
            if not active_is(w["address"]):
                log("REFUSED to type: Strata dialog not active")
                return False
            subprocess.run(["wtype", *step])
        return True
    def close(self):
        try:
            press(find_node(self._root(), "button", "Cancel file selection (Escape)"))
        except Exception:
            pass


ADAPTERS = {"flea": FleaAdapter, "gtk": GtkAdapter, "strata": StrataAdapter}


# ---------------------------------------------------------------- the strip (backend-agnostic)
class Chrome:
    def __init__(self, adapter):
        self.ad = adapter
        self.win = self.app = None
        self.presented = False
        self.seen_window = False
        self.lost_at = None
        self.last_folder = None
        self.shut = False

    # window rules: the strip (and a tiled-by-default dialog) map floating and in place, never tiled
    def rule(self, name, cls, on, geom=None):
        if on:
            x, y, w, h = geom
            code = ('hl.window_rule({ name = "%s", enabled = true, match = { class = "^%s$" }, float = true, size = "%d %d", move = "%d %d" })'
                    % (name, cls.replace(".", "\\\\."), w, h, x, y))
        else:
            code = 'hl.window_rule({ name = "%s", enabled = false })' % name
        log("rule %s %s -> %s" % (name, "on" if on else "off", hypr("eval", code)))

    def shutdown(self):
        if self.shut:
            return
        self.shut = True
        self.ad.kill()
        self.rule("ofolder-strip", STRIP_CLASS, False)
        if self.ad.needs_float:
            self.rule("ofolder-dialog", self.ad.window_class, False)

    def tick(self):
        if self.shut:
            return False
        w = self.ad.window()
        if w:
            self.seen_window, self.lost_at = True, None
            self.ensure_ready(w)
            if self.presented:
                self.dock(w)
                cur = self.ad.read_folder()
                self.now.set_text("Now: " + (cur or "…"))
                if cur and cur != self.last_folder:
                    log("FOLDER -> %s" % cur)
                    self.last_folder = cur
        elif self.seen_window:
            self.lost_at = self.lost_at or time.time()
            if time.time() - self.lost_at > 1.5 or not self.ad.alive():
                log("dialog gone; shutting down")
                self.shutdown()
                GLib.timeout_add(150, lambda: (self.app.quit(), False)[1])
                return False
        return True

    def ensure_ready(self, w):
        if self.presented:
            return
        if self.ad.needs_float and not w["floating"]:
            a = '"address:%s"' % w["address"]
            hypr("dispatch", 'hl.dsp.window.float({ action = "on", window = %s })' % a)
            hypr("dispatch", "hl.dsp.window.resize({ x = 1000, y = 640, relative = false, window = %s })" % a)
            b = sh.mon_bounds(w)
            if b:
                x = int((b[0] + b[2]) / 2 - (1000 + GAP + STRIP_W) / 2)
                y = int((b[1] + b[3]) / 2 - 320)
                hypr("dispatch", "hl.dsp.window.move({ x = %d, y = %d, relative = false, window = %s })" % (x, y, a))
            time.sleep(0.5)
            w = self.ad.window() or w
        x, y, sw, sh_, _ = strip_geom(w)
        self.rule("ofolder-strip", STRIP_CLASS, True, (x, y, sw, sh_))
        self.presented = True
        self.win.present()
        log("strip presented beside %s dialog at=%s size=%s floating=%s" % (self.ad.name, w["at"], w["size"], w["floating"]))

    def dock(self, p):
        s = find_class(STRIP_CLASS)
        if not s:
            return
        a = '"address:%s"' % s["address"]
        if not s["floating"]:
            hypr("dispatch", 'hl.dsp.window.float({ action = "on", window = %s })' % a)
        if s["workspace"]["id"] != p["workspace"]["id"]:
            hypr("dispatch", 'hl.dsp.window.move({ workspace = "%s", follow = false, window = %s })' % (p["workspace"]["name"], a))
        x, y, w, h, _ = strip_geom(p)
        if abs(s["at"][0] - x) > 1 or abs(s["at"][1] - y) > 1 or abs(s["size"][0] - w) > 1 or abs(s["size"][1] - h) > 1:
            hypr("dispatch", "hl.dsp.window.resize({ x = %d, y = %d, relative = false, window = %s })" % (w, h, a))
            hypr("dispatch", "hl.dsp.window.move({ x = %d, y = %d, relative = false, window = %s })" % (x, y, a))

    # --- actions the buttons call
    def wait_folder(self, want, timeout=3.0):
        t0 = time.time()
        while time.time() - t0 < timeout:
            if self.ad.read_folder() in want:
                return time.time() - t0
            time.sleep(0.02)

    def go(self, path):
        ok = self.ad.goto(path)
        t = self.wait_folder({path, os.path.realpath(path)}) if ok else None
        getattr(self.ad, "after_goto", lambda: None)()
        log("go %s -> %s" % (path[-60:], "arrived %.0fms" % (t * 1000) if t is not None else "NOT ARRIVED (issued=%s)" % ok))
        return t is not None

    def make_folder(self):
        base = self.ad.read_folder()
        if not base:
            log("REFUSED new folder: dialog folder unknown")
            return None
        for i in range(1, 100):
            p = os.path.join(base, "New folder" if i == 1 else "New folder %d" % i)
            try:
                os.mkdir(p)
                log("mkdir %s" % p)
                return p
            except FileExistsError:
                continue
            except OSError as e:
                log("FAILED mkdir in %s: %s" % (base, e))
                return None

    def new_folder_clicked(self):
        p = self.make_folder()
        if p:
            self.go(p)

    # --- UI
    def build(self, app):
        self.app = app
        self.win = Gtk.ApplicationWindow(application=app, title="ofolder strip")
        self.win.set_default_size(STRIP_W, 400)
        box = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=6, margin_top=10, margin_bottom=10, margin_start=10, margin_end=10)
        self.win.set_child(box)
        box.append(Gtk.Label(label="ofolder · %s" % self.ad.name, xalign=0))
        self.now = Gtk.Label(label="Now: …", xalign=0, ellipsize=Pango.EllipsizeMode.START, max_width_chars=26)
        box.append(self.now)
        box.append(Gtk.Separator())
        for label, path in (("Home", HOME), ("Work", HOME + "/Work"), ("Documents", HOME + "/Documents"), ("Downloads", HOME + "/Downloads")):
            b = Gtk.Button(label="→ " + label)
            b.connect("clicked", lambda _b, p=path: self.go(p))
            box.append(b)
        box.append(Gtk.Separator())
        nf = Gtk.Button(label="+ New folder")
        nf.connect("clicked", lambda _b: self.new_folder_clicked())
        box.append(nf)
        start = os.environ.get("SPIKE_START", HOME)
        if self.ad.needs_float:   # Strata maps tiled: float rule first, so it never reflows your layout
            b = sh.mon_bounds({"monitor": (hjson("monitors") or [{"id": 0}])[0]["id"]}) or (0, 0, 2000, 1000)
            x = int((b[0] + b[2]) / 2 - (1000 + GAP + STRIP_W) / 2); y = int((b[1] + b[3]) / 2 - 320)
            self.rule("ofolder-dialog", self.ad.window_class, True, (x, y, 1000, 640))
        self.ad.launch(start)
        GLib.timeout_add(200, self.tick)
        if os.environ.get("SPIKE_AUTO"):
            GLib.timeout_add(6000, self.auto)

    # --- unattended demo with evidence
    def shot(self, name):
        w, s = self.ad.window(), find_class(STRIP_CLASS)
        if not (w and s):
            return log("shot %s skipped (window=%s strip=%s)" % (name, bool(w), bool(s)))
        x, y = min(w["at"][0], s["at"][0]), min(w["at"][1], s["at"][1])
        x2 = max(w["at"][0] + w["size"][0], s["at"][0] + s["size"][0]); y2 = max(w["at"][1] + w["size"][1], s["at"][1] + s["size"][1])
        subprocess.run(["grim", "-g", "%d,%d %dx%d" % (x, y, x2 - x, y2 - y), os.path.join(OUT, name + ".png")])
        log("shot %s geom=%d,%d %dx%d" % (name, x, y, x2 - x, y2 - y))

    def check(self, name, ok, detail=""):
        log("%s %s %s" % ("PASS" if ok else "FAIL", name, detail))

    def auto(self):
        play = os.path.join(OUT, "playground", "sub")
        os.makedirs(play, exist_ok=True)

        def steps():
            w, s = self.ad.window(), find_class(STRIP_CLASS)
            self.check("strip docked beside dialog", bool(w and s) and s["at"][0] in (w["at"][0] + w["size"][0] + GAP, w["at"][0] - GAP - STRIP_W) and s["floating"],
                       "dialog at=%s size=%s | strip at=%s size=%s floating=%s" % (w and w["at"], w and w["size"], s and s["at"], s and s["size"], s and s["floating"]))
            self.shot("1-docked")
            yield 0.8
            self.check("reads start folder", self.ad.read_folder() == os.environ.get("SPIKE_START", HOME), "reading=%s" % self.ad.read_folder())
            yield 0.2
            self.check("strip button: go to a folder", self.go(play))
            yield 0.6
            created = self.make_folder()
            self.check("new folder created in the dialog's real folder", bool(created) and os.path.dirname(created) == self.ad.read_folder(), str(created and created[-50:]))
            if created:
                self.check("dialog entered the new folder", self.go(created))
            yield 0.8
            self.shot("2-new-folder")
            w = self.ad.window()
            a = '"address:%s"' % w["address"]
            hypr("dispatch", "hl.dsp.window.move({ x = 200, y = 100, relative = true, window = %s })" % a)
            yield 1.4
            w, s = self.ad.window(), find_class(STRIP_CLASS)
            ok = bool(w and s) and abs(s["at"][0] - strip_geom(w)[0]) <= 1 and abs(s["at"][1] - strip_geom(w)[1]) <= 1
            self.check("strip follows the dialog when it moves", ok, "dialog.at=%s strip.at=%s" % (w and w["at"], s and s["at"]))
            self.shot("3-after-move")
            yield 0.4
            self.check("strip button: back Home", self.go(HOME))
            rn = getattr(self.ad, "read_name", None)
            if rn:
                self.check("app's filename survived every move", rn() == "spike.txt", "name=%r" % rn())
            yield 0.5
            self.ad.close()
            yield 2.5
            self.check("dialog closed cleanly and strip exited", self.ad.window() is None and find_class(STRIP_CLASS) is None)
        gen = steps()
        def run():
            try:
                d = next(gen)
            except StopIteration:
                return False
            except Exception as e:
                log("AUTO CRASH %r" % e)
                return False
            GLib.timeout_add(int(d * 1000), run)
            return False
        run()
        return False


def main():
    name = os.environ.get("SPIKE_ADAPTER", "flea")
    chrome = Chrome(ADAPTERS[name]())
    app = Gtk.Application(application_id=STRIP_CLASS)
    app.connect("activate", chrome.build)
    quit_ = lambda: (chrome.shutdown(), app.quit(), False)[2]
    GLib.unix_signal_add(GLib.PRIORITY_DEFAULT, signal.SIGTERM, quit_)
    GLib.unix_signal_add(GLib.PRIORITY_DEFAULT, signal.SIGINT, quit_)
    atexit.register(chrome.shutdown)
    log("adapter=%s" % name)
    app.run([])
    chrome.shutdown()


if __name__ == "__main__":
    main()
