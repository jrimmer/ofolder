"""End to end, through the real portal path: a GTK4 program asks for a Save dialog, the real
xdg-desktop-portal frontend routes it to ofolder, which forwards to a real Flea (0.4 and 0.3) or GTK
dialog and docks its rails beside it. Run with `bin/e2e`.

Every test works in scratch folders inside a private nested Hyprland. Keys are typed only into a
window verified to be the dialog, and no real folder is read or written.
"""
import json
import os
import shutil
import signal
import subprocess
import sys
import tempfile
import time
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import flea03
from lib import OFOLDER, REPO, Host, Session, run, say, wait_for
from stack import Stack

STRIPS = ("ofolder.strip.left", "ofolder.strip.right")


def require_build():
    if not os.path.exists(OFOLDER):
        raise unittest.SkipTest("build first: cargo build --locked")


def strips_pids(found):
    return [p for p, _, _ in found]


class Flavour:
    """One backend behind ofolder. Subclasses set what differs."""

    name = ""
    chain = ""
    picker_class = ""
    flea_env = None  # extra environment for the backend (Flea 0.3's FLEA_BIN, FLEA_UI)
    ipc = None  # how to ask the picker where it is, independent of ofolder: "flea", "qs" or None
    pick_marker = ""  # what the picker's command line contains
    # Where the dialog's main button, and the "Replace" button of its overwrite prompt, are, from
    # the top-left of the dialog and its size. Read from screenshots of each.
    accept_at = staticmethod(lambda w, h: (w - 49, h - 56))
    replace_at = staticmethod(lambda w, h: (w - 49, h - 56))
    # Keys that do what the buttons do, for dialogs that do not act on a click sent to them.
    folder_keys = save_keys = replace_keys = None
    settle_before_keys = 1.0
    folder_select_unknown = False  # no way was found to confirm a folder
    multi_select_unknown = False  # no way was found to select several files
    filtered_needs_down = True

    @classmethod
    def setUpClass(cls):
        if cls is Flavour:
            raise unittest.SkipTest("base class")
        if cls.name.startswith("flea03"):
            env, why = flea03.ensure()
            if not env:
                raise unittest.SkipTest(why)
            cls.flea_env = env
        elif cls.name == "flea04" and not shutil.which("flea"):
            raise unittest.SkipTest("flea is not installed")
        elif cls.name == "strata" and not os.access(os.path.expanduser("~/.local/bin/strata"), os.X_OK):
            raise unittest.SkipTest("strata is not installed")
        elif cls.name == "gtk" and not os.path.exists("/usr/lib/xdg-desktop-portal-gtk"):
            raise unittest.SkipTest("xdg-desktop-portal-gtk is not installed")
        require_build()
        # A fresh nested session for every flavour, so nothing one leaves behind (clipboard
        # owners, helper processes, window state) can disturb the next.
        cls.host = Host()
        cls.session = Session(cls.host).__enter__()
        cls.s = cls.session
        try:
            cls.stack = Stack(cls.s, cls.chain, cls.flea_env).__enter__()
        except BaseException:
            cls.session.stop()  # unittest does not call tearDownClass when setUpClass fails
            raise
        cls.root = tempfile.mkdtemp(prefix="ofe2e-data.")
        cls.one = os.path.join(cls.root, "one")
        cls.two = os.path.join(cls.root, "two")
        for d in (cls.one, cls.two):
            os.makedirs(d)

    @classmethod
    def tearDownClass(cls):
        if cls is Flavour:
            return
        cls.stack.__exit__(None, None, None)
        cls.session.stop()
        shutil.rmtree(cls.root, ignore_errors=True)
        if getattr(cls.session, "layout_ok", True) is False:
            raise RuntimeError("the real monitors were not restored")

    # -- helpers ----------------------------------------------------------
    def setUp(self):
        self.s.run([OFOLDER, "resume"])
        self.clear_clipboard()
        # Each test starts with no favorites, recents or settings of its own making.
        home = self.s.env["HOME"]
        for f in (".config/ofolder/favorites.json", ".local/share/ofolder/recents.json", ".config/ofolder/settings.toml"):
            try:
                os.remove(os.path.join(home, f))
            except FileNotFoundError:
                pass
        self.proc = self.out = None

    def tearDown(self):
        self.close_dialog()
        self.s.run([OFOLDER, "resume"])

    def open_dialog(self, mode="save", folder=None):
        self.proc, self.out = self.stack.ask(mode, folder or self.one)
        dialog = wait_for(30, lambda: self.s.client(self.picker_class))
        self.assertTrue(dialog, "the dialog never opened:\n" + self.s.log("app"))
        return dialog

    def ready(self, folder=None):
        """Waits until the rails have read the dialog's folder. A command before that is rightly
        ignored, since the folder is not known yet."""
        want = folder or self.one
        self.assertTrue(self.folder_is(want, 20), "the rails never read the dialog's folder")

    def strips(self, limit=20):
        return wait_for(limit, lambda: all(self.s.client(c) for c in STRIPS))

    def answer(self, limit=15):
        return wait_for(limit, lambda: os.path.exists(self.out) and open(self.out).read())

    def close_dialog(self):
        """Cancel whatever is open and wait until the strips and the dialog are gone."""
        if self.proc is None:
            return
        self.cancel()
        wait_for(15, lambda: self.proc.poll() is not None)
        if self.proc.poll() is None:
            self.proc.terminate()
        self.assertTrue(
            wait_for(15, lambda: not any(self.s.client(c) for c in STRIPS + (self.picker_class,))),
            "windows left behind: %s" % [c["class"] for c in self.s.clients()],
        )
        self.proc = None

    def cancel(self):
        """The dialog's own way to say no: Escape, sent to the dialog's window and to no other."""
        self.s.press(self.picker_class, "Escape")

    def clear_clipboard(self):
        """Empties the clipboard so a read cannot return an earlier test's value. `wl-copy` may keep
        a helper alive that holds its pipes, so it is not waited on past a moment."""
        p = subprocess.Popen(
            ["wl-copy", "--clear"], env=self.s.env, stdin=subprocess.DEVNULL,
            stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, start_new_session=True,
        )
        try:
            p.wait(3)
        except subprocess.TimeoutExpired:
            p.kill()

    def confirm(self):
        """Press Return in the dialog until it answers. Flea's first key after it opens can arrive
        before the dialog counts the name as acceptable, so a press is repeated, one at a time, each
        sent to the dialog's own window."""
        for _ in range(5):
            if self.answer(0.2):
                break
            if not self.s.press(self.picker_class, "Return"):
                if self.answer(0.5):
                    break
                self.fail("the dialog closed before it could be confirmed")
            if self.answer(1.5):
                break
        return self.answer(3)

    def folder_is(self, folder, limit=1.0):
        """True once the rails read `folder` as the dialog's. The rails are asked at a person's pace:
        each copy-path raises a message window, and asking many times a second is not a use anyone
        has."""
        deadline = time.monotonic() + limit
        while True:
            self.s.run([OFOLDER, "do", "copy_path"])
            if self.clipboard(0.4) == folder:
                return True
            if time.monotonic() + 0.5 > deadline:
                return False
            time.sleep(0.5)

    def move(self, command, folder):
        """Sends one navigation command and waits for the dialog to be in `folder`. It is sent
        once: a command repeated after a slow read would move the dialog on a second time."""
        self.do(command)
        if self.ipc:
            # The dialog's own word first, which needs no window and no message.
            wait_for(12, lambda: self.picker_folder() == folder)
        if self.folder_is(folder, 12):
            return True
        self.s.run([OFOLDER, "do", "copy_path"])
        snap = ""
        if self.ipc:
            r = self.s.run([self.ipc, "ipc", "--pid", str(self.picker_pid()), "call", "fleapicker", "snapshot"], timeout=6)
            try:
                snap = json.loads([l for l in r.stdout.splitlines() if l.startswith("{")][-1]).get("path")
            except (ValueError, IndexError):
                snap = "(no snapshot: %r)" % r.stdout[:60]
        held = []
        pid = self.picker_pid()
        if pid:
            kids = [pid] + [int(x) for x in os.listdir("/proc") if x.isdigit() and self.parent_of(int(x)) == pid]
            for k in kids:
                try:
                    for fd in os.listdir("/proc/%d/fd" % k):
                        target = os.readlink("/proc/%d/fd/%s" % (k, fd))
                        if target.startswith("/tmp/ofe2e-data") and os.path.isdir(target):
                            held.append((k, target))
                except OSError:
                    pass
        strips = []
        for x in [int(d) for d in os.listdir("/proc") if d.isdigit()]:
            try:
                cmd = open("/proc/%d/cmdline" % x, "rb").read().replace(b"\0", b" ").decode()
                env = open("/proc/%d/environ" % x, "rb").read()
            except OSError:
                continue
            if ("XDG_RUNTIME_DIR=%s\0" % self.s.rt).encode() not in env:
                continue
            if "ofolder strip" in cmd or self.parent_of(x) in [int(y) for y in strips_pids(strips)]:
                state = open("/proc/%d/stat" % x).read().rsplit(")", 1)[1].split()[0]
                strips.append((x, state, cmd[:90]))
        def cpu(pid):
            out = {}
            for t in os.listdir("/proc/%d/task" % pid):
                try:
                    f = open("/proc/%d/task/%s/stat" % (pid, t)).read()
                    name = f[f.index("(") + 1:f.rindex(")")]
                    parts = f.rsplit(")", 1)[1].split()
                    out[t] = (name, parts[0], int(parts[11]) + int(parts[12]))
                except (OSError, ValueError, IndexError):
                    pass
            return out
        busy_threads = []
        for pid_, _, cmd_ in strips:
            if "ofolder strip" in cmd_:
                a = cpu(pid_)
                time.sleep(1)
                b = cpu(pid_)
                busy_threads = [(n, st, d) for t, (n, st, c) in b.items() for d in [c - a.get(t, (0, 0, c))[2]] if d > 0 or st == "R"]
        try:
            said = [l.strip() for l in open(os.path.join(self.s.dir, "bus.log")) if "ofolder strip:" in l and "command copy_path" not in l][-30:]
        except OSError:
            said = []
        self.diag = {"strip said": said, "processes": strips, "held": held, "threads_using_cpu_in_1s_(ticks)": busy_threads}
        self.last_state = "dialog model says %r, its drawn snapshot says %r, rails read %r, wanted %r" % (
            self.picker_folder(), snap, self.clipboard(1), folder)
        self.fail("%s did not bring the dialog to %s: %s\n%s\n%s" % (
            command, folder, self.last_state, getattr(self, "diag", ""), self.messages()))

    @staticmethod
    def parent_of(pid):
        try:
            return int(open("/proc/%d/stat" % pid).read().rsplit(")", 1)[1].split()[1])
        except (OSError, ValueError, IndexError):
            return None

    def messages(self):
        """What the rails told the user, for a failing test to show."""
        path = os.path.join(self.s.env["HOME"], ".local/state/ofolder/messages.log")
        try:
            lines = [l for l in open(path).read().splitlines() if "copy-path" not in l]
            return "\n".join(lines[-12:]) or "(nothing but copy-path notes)"
        except OSError:
            return "(no messages)"

    def do(self, command):
        r = self.s.run([OFOLDER, "do", command])
        self.assertEqual(r.returncode, 0, "ofolder do %s: %s" % (command, r.stderr))
        return r

    def clipboard(self, limit=5):
        def read():
            r = self.s.run(["wl-paste", "-n"], timeout=3)
            return r.stdout if r.returncode == 0 and r.stdout else None

        return wait_for(limit, read)

    def picker_folder(self):
        """Where the dialog says it is, asked of the dialog itself and not of ofolder."""
        if not self.ipc:
            return None
        pid = self.picker_pid()
        r = self.s.run([self.ipc, "ipc", "--pid", str(pid), "call", "fleapicker", "path"], timeout=6)
        # Quickshell can log a warning line before the answer; the answer is the last line.
        lines = r.stdout.strip().splitlines() if r.returncode == 0 else []
        return lines[-1] if lines else None

    def picker_pid(self):
        for p in [int(x) for x in os.listdir("/proc") if x.isdigit()]:
            try:
                env = open("/proc/%d/environ" % p, "rb").read()
                cmd = open("/proc/%d/cmdline" % p, "rb").read().replace(b"\0", b" ").decode()
            except OSError:
                continue
            if ("XDG_RUNTIME_DIR=%s\0" % self.s.rt).encode() in env and self.pick_marker in cmd:
                return p
        return None


class Rails(Flavour):
    """The checks every backend must pass."""

    def test_rails_dock_either_side_of_the_dialog_without_taking_focus(self):
        dialog = self.open_dialog()
        self.assertTrue(self.strips(), "no rails appeared:\n" + self.s.log("app"))
        left, right = (self.s.client(c) for c in STRIPS)
        dialog = self.s.client(self.picker_class)
        dx, dy = dialog["at"]
        dw, dh = dialog["size"]
        self.assertLessEqual(left["at"][0] + left["size"][0], dx + 1, "left rail is not left of the dialog")
        self.assertGreaterEqual(right["at"][0], dx + dw - 1, "right rail is not right of the dialog")
        self.assertEqual(left["at"][1], right["at"][1], "the rails differ in height position")
        self.assertLessEqual(abs(left["at"][1] - dy), dh, "the rails are not level with the dialog")
        # Never focus on its own: the keyboard stays with the dialog.
        self.assertEqual(self.s.active_class(), self.picker_class)

    def test_the_folder_the_dialog_is_in_reads_back_through_copy_path(self):
        self.open_dialog()
        self.assertTrue(self.strips())
        self.assertTrue(self.folder_is(self.one, 20), "the rails never read the dialog's folder")

    def test_parent_moves_the_dialog_and_the_new_folder_reads_back(self):
        self.open_dialog()
        self.assertTrue(self.strips())
        self.ready()
        want = self.root
        self.move("parent", want)
        if self.ipc:
            self.assertEqual(self.picker_folder(), want, "the dialog itself disagrees")

    def test_back_returns_to_the_folder_the_dialog_started_in(self):
        self.open_dialog()
        self.assertTrue(self.strips())
        self.ready()
        self.move("parent", self.root)
        self.move("back", self.one)

    def test_pressing_a_command_again_and_again_does_not_bring_the_rails_down(self):
        """Found by this suite: a superseded message window was removed inside the update that
        superseded it, and the rails panicked on the fourth or fifth quick press."""
        self.open_dialog()
        self.assertTrue(self.strips())
        for i in range(40):
            r = self.s.run([OFOLDER, "do", "copy_path"])
            self.assertEqual(r.returncode, 0, "press %d: no rail answered; they are gone: %s" % (
                i, [c["class"] for c in self.s.clients()]))
            wait_for(0.05, lambda: False)
        self.assertTrue(self.strips(1), "the rails are gone after quick presses")
        self.assertNotIn("panicked", self.s.log("bus"), "a strip panicked")

    def test_the_rails_use_little_cpu_when_nothing_is_happening(self):
        """A hot loop in the rails would starve their window and make them read stale. The debug
        build is far slower and always fails this, so it is judged on the release build only."""
        if "release" not in OFOLDER:
            self.skipTest("judged on the release build: run bin/e2e --release")
        self.open_dialog()
        self.assertTrue(self.strips())
        self.ready()
        time.sleep(6)  # let the opening settle, and any message window go
        pid = next(
            (p for p in (int(d) for d in os.listdir("/proc") if d.isdigit()) if self.is_strip(p)), None
        )
        self.assertTrue(pid, "no strip process")

        def ticks():
            f = open("/proc/%d/stat" % pid).read().rsplit(")", 1)[1].split()
            return int(f[11]) + int(f[12])

        a = ticks()
        time.sleep(10)
        used = (ticks() - a) * 100.0 / (10 * os.sysconf("SC_CLK_TCK"))
        say("idle CPU of the rails (%s): %.2f%% of a core over 10 s" % (self.name, used))
        self.assertLess(used, 25, "the rails used %.1f%% of a core while idle" % used)

    def is_strip(self, pid):
        try:
            cmd = open("/proc/%d/cmdline" % pid, "rb").read()
            env = open("/proc/%d/environ" % pid, "rb").read()
        except OSError:
            return False
        return b"ofolder\0strip" in cmd and ("XDG_RUNTIME_DIR=%s\0" % self.s.rt).encode() in env

    def test_cancelling_leaves_nothing_behind_and_the_app_hears_it(self):
        self.open_dialog()
        self.assertTrue(self.strips())
        self.cancel()
        ans = self.answer()
        self.assertTrue(ans and ans.startswith("CANCELLED"), "the app heard: %r" % ans)
        self.assertTrue(
            wait_for(15, lambda: not any(self.s.client(c) for c in STRIPS)), "the rails stayed after the dialog closed"
        )
        binds = json.loads(self.s.hypr("binds", "-j").stdout or "[]")
        self.assertFalse([b for b in binds if "ofolder" in json.dumps(b)], "key bindings were left behind")
        self.proc = None

    def test_confirming_returns_the_chosen_file(self):
        self.open_dialog()
        self.assertTrue(self.strips())
        ans = self.confirm()
        self.assertEqual(ans, "SAVED " + os.path.join(self.one, "e2e.txt"))
        self.proc = None

    def test_a_confirmed_dialog_is_remembered_and_a_cancelled_one_is_not(self):
        recents = os.path.join(self.s.env["HOME"], ".local/share/ofolder/recents.json")
        self.open_dialog()
        self.assertTrue(self.strips())
        self.cancel()
        self.answer()
        self.assertTrue(wait_for(15, lambda: not self.s.client(self.picker_class)))
        self.assertFalse(
            os.path.exists(recents) and self.one in open(recents).read(), "a cancelled dialog was recorded"
        )
        self.proc = None
        self.open_dialog()
        self.assertTrue(self.strips())
        self.assertTrue(self.confirm())
        self.assertTrue(
            wait_for(10, lambda: os.path.exists(recents) and self.one in open(recents).read()),
            "a confirmed dialog was not recorded",
        )
        self.proc = None

    def test_status_reports_the_chain_and_warns_when_ofolder_is_dropped_from_it(self):
        report = self.s.run([OFOLDER, "status"]).stdout
        self.assertIn("file chooser chain: %s (ofolder first)" % self.chain, report)
        conf = os.path.join(self.s.env["HOME"], ".config/xdg-desktop-portal/hyprland-portals.conf")
        original = open(conf).read()
        try:
            open(conf, "w").write("[preferred]\norg.freedesktop.impl.portal.FileChooser=flea;gtk\n")
            report = self.s.run([OFOLDER, "status"]).stdout
            self.assertIn("warning: ofolder is not in the file chooser chain", report)
        finally:
            open(conf, "w").write(original)

    def test_paused_means_the_dialog_is_left_untouched(self):
        self.s.run([OFOLDER, "pause"])
        self.open_dialog()
        self.assertFalse(self.strips(6), "rails appeared while ofolder was paused")
        self.assertTrue(self.s.client(self.picker_class))

    def test_the_rails_follow_the_dialog_when_it_moves(self):
        dialog = self.open_dialog()
        self.assertTrue(self.strips())
        before = self.s.client("ofolder.strip.left")["at"]
        self.s.hypr("dispatch", 'hl.dsp.window.move({ x = %d, y = %d, window = "address:%s" })' % (
            dialog["at"][0] + 60, dialog["at"][1] + 40, dialog["address"]))
        moved = wait_for(10, lambda: self.s.client("ofolder.strip.left")["at"] != before)
        self.assertTrue(moved, "the left rail did not follow the dialog")


def write_png(path):
    """A 2 by 2 PNG, made here so the suite needs no imaging library."""
    import struct
    import zlib

    def chunk(kind, data):
        body = kind + data
        return struct.pack(">I", len(data)) + body + struct.pack(">I", zlib.crc32(body))

    rows = b"".join(b"\x00" + bytes([200, 40, 40]) * 2 for _ in range(2))
    png = b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", 2, 2, 8, 2, 0, 0, 0))
    png += chunk(b"IDAT", zlib.compress(rows)) + chunk(b"IEND", b"")
    with open(path, "wb") as f:
        f.write(png)


MENU = "ofolder.strip.menu"
HEAD_LIST = 44  # a menu's fixed top, then rows of 48; recents carry a switch above their rows
HEAD_RECENT = 76
ROW = 48


class Features:
    """What the rails offer beyond moving: favorites, places, recents, new folder, find, settings,
    preview. A menu is opened with its command and an item is chosen with a real click."""

    def home(self):
        return self.s.env["HOME"]

    def data(self, *parts):
        return os.path.join(self.home(), ".local/share/ofolder", *parts)

    def config(self, *parts):
        return os.path.join(self.home(), ".config/ofolder", *parts)

    def favorites_text(self):
        try:
            return open(self.config("favorites.json")).read()
        except OSError:
            return ""

    def seed_recents(self, folders=(), files=()):
        os.makedirs(self.data(), exist_ok=True)
        now = int(time.time())
        doc = {
            "folders": [{"path": f, "apps": [], "at": now - i} for i, f in enumerate(folders)],
            "files": [{"path": f, "apps": [], "at": now - i} for i, f in enumerate(files)],
        }
        open(self.data("recents.json"), "w").write(json.dumps(doc))

    def open_menu(self, command):
        # A message ("Path copied") lives in a window of the same class as the menus, so it must
        # be gone before a menu's window can be told from it.
        self.assertTrue(wait_for(15, lambda: not self.s.client(MENU)), "a message window would not go away")
        self.do(command)
        menu = wait_for(10, lambda: self.s.client(MENU))
        self.assertTrue(menu, "%s opened no menu" % command)
        return menu

    def choose(self, command, row, head=HEAD_LIST):
        """Opens the menu for `command` and clicks its row `row`."""
        self.open_menu(command)
        time.sleep(0.5)
        self.assertTrue(self.s.click(MENU, 150, head + row * ROW + ROW // 2), "the click did not land")

    def start(self, folder=None, mode="save"):
        self.open_dialog(mode=mode, folder=folder)
        self.assertTrue(self.strips())
        self.ready(folder)

    def test_a_folder_is_added_to_the_favorites_and_taken_off_again(self):
        self.start()
        self.assertNotIn(self.one, self.favorites_text())
        self.do("favorite")
        self.assertTrue(wait_for(10, lambda: self.one in self.favorites_text()), "the folder was not added")
        time.sleep(1.2)  # the rails ignore a second press while the first is being shown
        self.do("favorite")
        self.assertTrue(
            wait_for(10, lambda: self.one not in self.favorites_text()), "the folder was not taken off again"
        )

    def test_a_favorite_chosen_from_its_menu_takes_the_dialog_there(self):
        self.start()
        self.do("favorite")
        self.assertTrue(wait_for(10, lambda: self.one in self.favorites_text()))
        self.move("parent", self.root)
        self.choose("favorites", 0)
        self.assertTrue(self.folder_is(self.one, 12), "the favorite did not take the dialog to its folder")

    def shot(self, name):
        """A picture of the menu window, kept when E2E_SHOTS names a folder, to see what a click saw."""
        folder = os.environ.get("E2E_SHOTS")
        menu = self.s.client(MENU)
        if folder and menu:
            self.s.run(["grim", "-g", "%d,%d %dx%d" % (*menu["at"], *menu["size"]), os.path.join(folder, name + ".png")])
            open(os.path.join(folder, name + ".txt"), "w").write(json.dumps(menu["at"]) + json.dumps(menu["size"]))

    def seed_favorites(self, *entries):
        os.makedirs(self.config(), exist_ok=True)
        open(self.config("favorites.json"), "w").write(json.dumps({"favorites": list(entries)}))

    def favorite_paths(self):
        try:
            return [f["path"] for f in json.load(open(self.config("favorites.json")))["favorites"]]
        except (OSError, ValueError, KeyError):
            return []

    def row_menu(self, button_xy):
        """Opens the favorites menu, then the first row's own menu (the dots), then presses a button
        in it. The positions are those of a 300 pixel menu with its first row at the top."""
        first = self.open_menu("favorites")["size"][1]
        time.sleep(1.0)
        # A click that arrives before the window takes input is lost, so it is repeated until the
        # row's menu has opened, which makes the window taller.
        for _ in range(4):
            self.assertTrue(self.s.click(MENU, 283, 52), "the dots did not click")
            if wait_for(2, lambda: self.s.client(MENU)["size"][1] > first):
                break
        else:
            self.fail("the row's menu never opened")
        self.shot("row-menu")
        self.assertTrue(self.s.click(MENU, *button_xy), "the row's button did not click")

    def test_a_favorite_is_removed_from_its_row_menu(self):
        self.seed_favorites({"path": self.one}, {"path": self.two, "name": "Second"})
        self.start()
        self.row_menu((145, 120))  # Remove favorite
        self.assertTrue(
            wait_for(8, lambda: self.favorite_paths() == [self.two]),
            "the favorite was not removed: %s" % self.favorite_paths(),
        )

    def test_a_favorite_is_moved_down_the_list_from_its_row_menu(self):
        self.seed_favorites({"path": self.one}, {"path": self.two})
        self.start()
        self.row_menu((263, 120))  # Down
        self.assertTrue(
            wait_for(8, lambda: self.favorite_paths() == [self.two, self.one]),
            "the favorite did not move: %s" % self.favorite_paths(),
        )

    def test_a_favorite_is_renamed_from_its_row_menu(self):
        # Two favorites: with one the menu window is too short to show the Rename button.
        self.seed_favorites({"path": self.one}, {"path": self.two})
        self.start()
        self.row_menu((51, 151))  # Rename
        time.sleep(1.0)
        # A click would have focused the menu; a click sent to a window does not, so it is done here.
        self.assertTrue(self.s.focus(MENU), "the menu would not take the keyboard")
        # The field opens with the old name selected; clear it, then type the new one.
        for _ in range(8):
            self.s.type_text(MENU, key="BackSpace")
        self.assertTrue(self.s.type_text(MENU, "Work"))
        self.assertTrue(self.s.type_text(MENU, key="Return"))
        self.assertTrue(
            wait_for(8, lambda: '"name": "Work"' in self.favorites_text()),
            "the new name was not saved:\n" + self.favorites_text(),
        )

    def test_a_shortcut_set_in_the_settings_is_bound_while_the_dialog_is_open_and_gone_after(self):
        """Pressing the combination is not tested: a virtual keyboard does not reach the nested
        compositor's binds, not even a plain one. The command it runs is `ofolder do`, tested above."""
        os.makedirs(self.config(), exist_ok=True)
        open(self.config("settings.toml"), "w").write('[shortcuts]\nfavorite = "SUPER+ALT+G"\n')
        self.start()
        ours = lambda: [b for b in json.loads(self.s.hypr("binds", "-j").stdout or "[]") if "ofolder" in json.dumps(b)]
        bound = wait_for(10, ours)
        self.assertTrue(bound, "the shortcut was not bound")
        self.assertEqual((bound[0]["modmask"], bound[0]["key"]), (72, "G"), "bound to the wrong keys")
        self.close_dialog()
        self.assertTrue(wait_for(10, lambda: not ours()), "the binding was left behind after the dialog closed")

    def test_a_recent_folder_chosen_from_its_menu_takes_the_dialog_there(self):
        self.seed_recents(folders=[self.two])
        self.start()
        self.choose("recent_folders", 0, HEAD_RECENT)
        self.assertTrue(self.folder_is(self.two, 12), "the recent folder did not take the dialog there")

    def test_a_recent_file_chosen_from_its_menu_takes_the_dialog_to_its_folder(self):
        note = os.path.join(self.two, "note.txt")
        open(note, "w").write("x")
        self.seed_recents(files=[note])
        self.start()
        self.choose("recent_files", 0, HEAD_RECENT)
        self.assertTrue(self.folder_is(self.two, 12), "the recent file did not take the dialog to its folder")

    def test_a_place_chosen_from_its_menu_takes_the_dialog_there(self):
        self.start()
        self.open_menu("places")
        time.sleep(0.5)
        self.assertTrue(self.s.click(MENU, 150, HEAD_LIST + ROW // 2))
        self.assertTrue(self.folder_is(self.home(), 12), "the first place was not the home folder or did not open")

    def open_file_manager(self, folder):
        """A Flea file-manager window on `folder`, started in the session like a person would."""
        if self.flea_env:
            return self.s.spawn([self.flea_env["FLEA_BIN"], "--gui", folder], env=self.flea_env, name="manager")
        return self.s.spawn(["flea", "--gui", folder], name="manager")

    def find_goes_to(self, word, folder, limit=12):
        """Types `word` in the Find box and presses Enter, until the dialog is in `folder`."""
        self.open_menu("find")
        self.assertTrue(wait_for(5, lambda: self.s.active_class() == MENU), "find did not take the keyboard")
        self.assertTrue(self.s.type_text(MENU, word))
        time.sleep(1.5)
        for _ in range(3):
            if self.s.client(MENU) and self.s.active_class() == MENU:
                self.s.type_text(MENU, key="Return")
            if self.folder_is(folder, 4):
                return True
        return self.folder_is(folder, limit - 6)

    def test_the_folder_of_an_open_file_manager_window_is_offered_and_goes_when_the_window_closes(self):
        """R-25. The folder has a name nothing else knows (no favorite, recent or place), so Find
        can only offer it if the open window is the source."""
        target = self.made("zzopenwin")
        window = self.open_file_manager(target)
        self.assertTrue(
            wait_for(40, lambda: any(c["class"] == "com.thisisgm.flea" for c in self.s.clients())),
            "the file manager window never opened",
        )
        time.sleep(4)
        self.start()
        # The places are read every few seconds, so the window is found within about five.
        time.sleep(6)
        self.assertTrue(self.find_goes_to("zzopenwin", target), "the open window's folder was not offered")
        window.terminate()
        self.assertTrue(
            wait_for(20, lambda: not any(c["class"] == "com.thisisgm.flea" for c in self.s.clients())),
            "the file manager window did not close",
        )
        self.move("parent", self.root)
        time.sleep(8)  # a closed window is dropped at the next read
        self.assertFalse(
            self.find_goes_to("zzopenwin", target, limit=8),
            "a closed window's folder was still offered",
        )

    def test_a_new_folder_is_made_from_its_prompt_and_the_dialog_goes_into_it(self):
        self.start()
        self.open_menu("new_folder")
        made = os.path.join(self.one, "made")
        self.assertTrue(wait_for(5, lambda: self.s.active_class() == MENU), "the prompt did not take the keyboard")
        self.assertTrue(self.s.type_text(MENU, "made"))
        self.assertTrue(self.s.type_text(MENU, key="Return"))
        self.assertTrue(wait_for(10, lambda: os.path.isdir(made)), "the folder was not made")
        self.assertTrue(
            self.folder_is(made, 12),
            "the dialog did not go into the new folder; the rails read %r\n%s" % (self.clipboard(1), self.messages()),
        )

    def test_find_ranks_the_folders_it_knows_and_enter_goes_to_the_best(self):
        """The box ranks favorites, recents and places as you type; the search below the folder
        is a separate button and is not exercised here."""
        self.seed_recents(folders=[self.two, self.one])
        self.start(self.root)
        self.open_menu("find")
        self.assertTrue(wait_for(5, lambda: self.s.active_class() == MENU), "find did not take the keyboard")
        self.assertTrue(self.s.type_text(MENU, "two"))
        time.sleep(1.5)
        # Enter acts on the highlighted match; if the list had not ranked yet it does nothing, so it
        # is pressed again while the menu is still up.
        for _ in range(3):
            if self.s.client(MENU) and self.s.active_class() == MENU:
                self.assertTrue(self.s.type_text(MENU, key="Return"))
            if self.folder_is(self.two, 4):
                break
        self.assertTrue(self.folder_is(self.two, 6), "find did not take the dialog to the match")

    # The settings window: pane buttons and controls are found by their place in the window, which
    # is fixed at 780 by 560. Each change is saved to settings.toml at once, so that is what is read.
    SETTINGS = "ofolder.strip.settings"

    def settings_text(self):
        try:
            return open(self.config("settings.toml")).read()
        except OSError:
            return ""

    def open_settings(self):
        self.start()
        self.do("settings")
        self.assertTrue(wait_for(10, lambda: self.s.client(self.SETTINGS)), "no settings window")
        time.sleep(1.0)

    def click_settings(self, x, y):
        self.assertTrue(self.s.click(self.SETTINGS, x, y), "the click did not land")
        time.sleep(0.6)

    # The guard: a switch in the General pane at (300, 409); turning it off asks first, with
    # "Keep it on" at (475, 390) and "Turn it off" at (555, 390).
    def guard_files(self):
        data = os.path.join(self.home(), ".local/share/ofolder/ofolder-guard.sh")
        units = os.path.join(self.home(), ".config/systemd/user")
        return os.path.isfile(data) and os.path.isfile(os.path.join(units, "ofolder-guard.path"))

    def remove_guard_files(self):
        for f in (".local/share/ofolder/ofolder-guard.sh", ".config/systemd/user/ofolder-guard.path",
                  ".config/systemd/user/ofolder-guard.service"):
            try:
                os.remove(os.path.join(self.home(), f))
            except FileNotFoundError:
                pass

    def test_the_guard_is_put_in_place_on_the_first_start_because_it_is_on_by_default(self):
        self.remove_guard_files()
        self.start()  # a dialog opens, so the portal starts, and it makes the guard match the setting
        self.assertTrue(wait_for(15, self.guard_files), "the guard was not installed by default")

    def test_turning_the_guard_off_asks_first_and_keeping_it_on_changes_nothing(self):
        self.open_settings()
        self.assertTrue(wait_for(15, self.guard_files))
        self.click_settings(300, 409)  # the guard's switch: this asks
        time.sleep(0.8)
        self.assertNotIn("guard = false", self.settings_text(), "it turned off without asking")
        self.click_settings(475, 390)  # Keep it on
        time.sleep(1.0)
        self.assertNotIn("guard = false", self.settings_text())
        self.assertTrue(self.guard_files(), "keeping it on removed the guard")

    def test_escape_in_the_question_keeps_the_guard_on_and_leaves_the_settings_open(self):
        self.open_settings()
        self.assertTrue(wait_for(15, self.guard_files))
        self.click_settings(300, 409)
        time.sleep(0.8)
        self.assertTrue(self.s.press(self.SETTINGS, "Escape"))
        time.sleep(1.0)
        self.assertTrue(self.s.client(self.SETTINGS), "Escape closed the settings window instead of the question")
        self.assertNotIn("guard = false", self.settings_text())
        self.assertTrue(self.guard_files())

    def test_the_guard_is_turned_off_after_confirming_and_back_on_with_one_click(self):
        self.open_settings()
        self.assertTrue(wait_for(15, self.guard_files))
        self.click_settings(300, 409)
        time.sleep(0.8)
        self.click_settings(555, 390)  # Turn it off
        self.assertTrue(wait_for(10, lambda: "guard = false" in self.settings_text()), self.settings_text())
        self.assertTrue(wait_for(10, lambda: not self.guard_files()), "the guard was not taken away")
        # doctor says it is off as chosen, not a problem
        report = self.s.run([OFOLDER, "doctor"]).stdout
        self.assertIn("the guard is turned off in the settings, as chosen", report)
        # Turning it back on needs no question.
        self.click_settings(300, 409)
        self.assertTrue(wait_for(10, lambda: "guard = true" in self.settings_text()), self.settings_text())
        self.assertTrue(wait_for(10, self.guard_files), "the guard was not put back")

    # The recents controls in the General pane: age chips at y 213 (30 days x 406, 90 days x 472,
    # 1 year x 531, never x 586) and "Forget all recents..." at (266, 311); its question has
    # "Keep them" at (459, 343) and "Forget them" at (547, 343).
    def recents_doc(self):
        try:
            return json.load(open(self.data("recents.json")))
        except (OSError, ValueError):
            return None

    def test_the_age_after_which_recents_are_forgotten_is_chosen_with_chips(self):
        self.open_settings()
        self.click_settings(406, 213)  # 30 days
        self.assertTrue(wait_for(8, lambda: "recents_days = 30" in self.settings_text()), self.settings_text())
        self.click_settings(586, 213)  # never
        self.assertTrue(wait_for(8, lambda: "recents_days = 0" in self.settings_text()), self.settings_text())

    def test_forgetting_all_recents_asks_first_and_then_empties_them_and_keeps_the_favorites(self):
        self.seed_recents(folders=[self.one, self.two], files=[os.path.join(self.two, "a.txt")])
        self.seed_favorites({"path": self.one})
        self.open_settings()
        self.click_settings(266, 311)  # Forget all recents...
        time.sleep(0.8)
        self.click_settings(459, 343)  # Keep them
        time.sleep(1.0)
        doc = self.recents_doc()
        self.assertTrue(doc and len(doc["folders"]) == 2 and len(doc["files"]) == 1, "keeping them changed the recents: %s" % doc)
        self.click_settings(266, 311)
        time.sleep(0.8)
        self.assertTrue(self.s.press(self.SETTINGS, "Escape"))
        time.sleep(0.8)
        self.assertTrue(self.s.client(self.SETTINGS), "Escape closed the window instead of the question")
        self.assertEqual(len(self.recents_doc()["folders"]), 2, "Escape forgot the recents")
        self.click_settings(266, 311)
        time.sleep(0.8)
        self.click_settings(547, 343)  # Forget them
        self.assertTrue(
            wait_for(10, lambda: (self.recents_doc() or {}).get("folders") == [] and self.recents_doc().get("files") == []),
            "the recents were not emptied: %s" % self.recents_doc(),
        )
        self.assertIn(self.one, self.favorites_text(), "forgetting touched the favorites")
        self.assertEqual(os.stat(self.data("recents.json")).st_mode & 0o777, 0o600)

    def test_ofolder_forget_from_the_command_line_names_what_it_removed_and_leaves_favorites(self):
        self.seed_recents(folders=[self.one], files=[os.path.join(self.two, "b.txt")])
        self.seed_favorites({"path": self.two})
        r = self.s.run([OFOLDER, "forget"])
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertIn(self.one, r.stdout)
        self.assertIn("b.txt", r.stdout)
        self.assertEqual(self.recents_doc()["folders"], [])
        self.assertIn(self.two, self.favorites_text())

    def test_what_ofolder_keeps_about_you_is_readable_by_you_only(self):
        self.start()
        self.assertTrue(self.confirm())
        self.assertTrue(wait_for(10, lambda: self.recents_doc() is not None), "nothing was recorded")
        self.proc = None
        for name in ("recents.json",):
            self.assertEqual(os.stat(self.data(name)).st_mode & 0o777, 0o600, name)

    def test_a_save_in_a_sensitive_folder_is_not_recorded_by_default(self):
        secret = os.path.join(self.home(), ".ssh", "keys-for-work")
        os.makedirs(secret, exist_ok=True)
        self.start(secret)
        self.assertTrue(self.confirm(), "the dialog did not confirm")
        self.proc = None
        time.sleep(3)
        self.assertNotIn(".ssh", json.dumps(self.recents_doc() or {}), "a folder under ~/.ssh was recorded")

    def test_a_general_setting_is_saved_when_it_is_switched(self):
        self.open_settings()
        self.assertNotIn("recents_enabled = false", self.settings_text())
        self.click_settings(221, 115)  # Remember recent folders and files
        self.assertTrue(
            wait_for(8, lambda: "recents_enabled = false" in self.settings_text()),
            "switching it off was not saved:\n" + self.settings_text(),
        )
        self.click_settings(221, 115)
        self.assertTrue(
            wait_for(8, lambda: "recents_enabled = true" in self.settings_text()),
            "switching it on again was not saved:\n" + self.settings_text(),
        )

    def test_the_recents_count_is_stepped_down_and_saved(self):
        self.open_settings()
        self.click_settings(388, 152)  # the minus beside "Recent items kept"
        self.assertTrue(
            wait_for(8, lambda: "recents_max" in self.settings_text() and "recents_max = 20" not in self.settings_text()),
            "the count was not lowered:\n" + self.settings_text(),
        )

    def test_the_preview_size_cap_is_chosen_in_its_pane(self):
        self.open_settings()
        self.click_settings(44, 97)  # the Preview pane
        self.click_settings(453, 240)  # 32 MB
        self.assertTrue(
            wait_for(8, lambda: "max_mb = 32" in self.settings_text()),
            "the cap was not saved:\n" + self.settings_text(),
        )

    def test_hiding_a_rail_icon_is_saved_and_the_default_layout_brings_it_back(self):
        self.open_settings()
        self.click_settings(44, 63)  # the Rails pane
        before = self.settings_text()
        self.click_settings(725, 164)  # Hide, on Back
        self.assertTrue(
            wait_for(8, lambda: self.settings_text() != before), "hiding Back was not saved:\n" + self.settings_text()
        )
        hidden = self.settings_text()
        self.click_settings(684, 77)  # Reset to default layout
        self.assertTrue(
            wait_for(8, lambda: self.settings_text() != hidden),
            "the default layout was not restored:\n" + self.settings_text(),
        )

    def test_the_preview_shows_while_an_image_is_selected_and_the_shortcut_hides_it(self):
        pics = os.path.join(self.root, "pics")
        os.makedirs(pics, exist_ok=True)
        write_png(os.path.join(pics, "a.png"))
        # An Open dialog, where choosing a file is the point, and one Down to select the image
        # in the list when the dialog has not already.
        self.start(pics, mode="open")
        PREVIEW = "ofolder.strip.preview"
        if not wait_for(4, lambda: self.s.client(PREVIEW)):
            self.s.press(self.picker_class, "Down")
        self.assertTrue(wait_for(15, lambda: self.s.client(PREVIEW)), "no preview for the selected image")
        dialog = self.s.client(self.picker_class)
        shown = self.s.client(PREVIEW)
        self.assertGreaterEqual(shown["at"][1], dialog["at"][1] + dialog["size"][1], "the preview is not under the dialog")
        self.do("preview")
        self.assertTrue(wait_for(10, lambda: not self.s.client(PREVIEW)), "the shortcut did not hide the preview")

    def test_a_folder_with_no_image_selected_shows_no_preview(self):
        plain = os.path.join(self.root, "plain")
        os.makedirs(plain, exist_ok=True)
        open(os.path.join(plain, "a.txt"), "w").write("x")
        self.start(plain, mode="open")
        self.s.press(self.picker_class, "Down")
        self.assertFalse(wait_for(5, lambda: self.s.client("ofolder.strip.preview")), "a preview for a text file")

    def test_the_settings_window_opens_and_closes(self):
        self.start()
        self.do("settings")
        self.assertTrue(wait_for(10, lambda: self.s.client("ofolder.strip.settings")), "no settings window")
        self.do("settings")
        self.assertTrue(
            wait_for(10, lambda: not self.s.client("ofolder.strip.settings")), "the settings window did not close"
        )


class RealApps:
    """The shapes real applications ask for: several files, a folder, a type filter, a file that
    already exists, awkward and deep folder names, and a folder reached through a link."""

    def ask(self, mode, folder, name="e2e.txt"):
        self.proc, self.out = self.stack.ask(mode, folder, name)
        self.assertTrue(wait_for(30, lambda: self.s.client(self.picker_class)), "the dialog never opened")
        self.assertTrue(self.strips(), "no rails:\n" + self.s.log("app"))
        # A dialog driven by keys must have settled before the first one: under load a key that
        # arrives early is lost, and the sequences below depend on where focus is.
        time.sleep(self.settle_before_keys)

    def made(self, *parts):
        path = os.path.join(self.root, *parts)
        os.makedirs(path, exist_ok=True)
        return path

    def test_several_files_chosen_in_an_open_dialog_all_come_back(self):
        if self.multi_select_unknown:
            self.skipTest("no way was found to select several files in this dialog by keyboard")
        folder = self.made("many")
        names = ["a.txt", "b.txt", "c.txt"]
        for n in names:
            open(os.path.join(folder, n), "w").write("x")
        self.ask("multi", folder)
        self.ready(folder)
        self.s.press(self.picker_class, "Down")
        self.s.press(self.picker_class, "A", "CTRL")  # select all
        answer = self.confirm()
        self.assertTrue(answer and answer.startswith("OPENED "), answer)
        got = sorted(os.path.basename(p) for p in answer[len("OPENED "):].split("|"))
        self.assertEqual(got, names, "not every selected file came back")

    def flea_button(self, *names):
        """Where Flea 0.4 says one of its buttons is, from its own `controls` call (window coordinates)."""
        r = self.s.run([self.ipc, "ipc", "--pid", str(self.picker_pid()), "call", "fleapicker", "controls"], timeout=6)
        lines = [l for l in r.stdout.splitlines() if l.startswith("[{")]
        try:
            controls = json.loads(lines[-1])
        except (ValueError, IndexError):
            return None
        for c in controls:
            if c.get("name") in names and c.get("visible") and c.get("enabled"):
                x, y = (int(v) for v in c["centre"].split())
                return x, y
        return None

    def accept(self, kind):
        """Presses the dialog's main button for `kind` ("folder": Select, "save": Save, "replace":
        the overwrite prompt's Replace). Where a dialog can be reached by keys they are used, each
        sent to the dialog's own window; else the button is clicked where it is."""
        keys = getattr(self, kind + "_keys")
        if keys:
            for key in keys:
                self.assertTrue(self.s.press(self.picker_class, key), "the dialog went away")
                time.sleep(0.7)
            return
        names = {"folder": ("_Select", "Select"), "save": ("_Save", "Save"), "replace": ("Replace", "_Replace")}[kind]
        d = self.s.client(self.picker_class)
        found = self.flea_button(*names) if self.ipc == "flea" else None
        where = self.replace_at if kind == "replace" else self.accept_at
        x, y = found or where(*d["size"])
        self.assertTrue(self.s.click_at(self.picker_class, x, y), "the button did not click")

    def test_a_folder_chosen_in_a_folder_dialog_comes_back(self):
        if self.folder_select_unknown:
            self.skipTest("no way was found to confirm a folder in this dialog")
        outer = self.made("pick")
        self.made("pick", "inner")
        self.ask("folder", outer)
        self.ready(outer)
        self.accept("folder")
        answer = self.answer()
        self.assertTrue(answer and answer.startswith("OPENED "), answer)
        chosen = answer[len("OPENED "):]
        self.assertTrue(os.path.isdir(chosen) and chosen.startswith(outer), "not the folder shown: %s" % chosen)

    def test_a_dialog_that_offers_only_text_files_still_gets_the_rails_and_returns_one(self):
        folder = self.made("filtered")
        open(os.path.join(folder, "a.txt"), "w").write("x")
        open(os.path.join(folder, "b.png"), "w").write("x")
        self.ask("filtered", folder)
        self.ready(folder)
        if self.filtered_needs_down:
            self.s.press(self.picker_class, "Down")
        answer = self.confirm()
        self.assertTrue(answer and answer.startswith("OPENED ") and answer.endswith(".txt"), answer)

    def test_saving_over_a_file_that_exists_asks_and_then_returns_the_file(self):
        folder = self.made("over")
        existing = os.path.join(folder, "e2e.txt")
        open(existing, "w").write("old")
        self.ask("save", folder)
        self.ready(folder)
        self.accept("save")
        time.sleep(1.5)
        self.assertFalse(self.answer(0.5), "the dialog answered before the overwrite was confirmed")
        self.accept("replace")
        self.assertEqual(self.answer(), "SAVED " + existing, "the overwrite was not carried through")
        self.assertEqual(open(existing).read(), "old", "neither ofolder nor the dialog may write the file itself")

    def test_a_folder_with_spaces_quotes_and_unicode_in_its_name_reads_back_and_is_left_and_re_entered(self):
        odd = self.made("weird ünï \"q\" 'a' & ; $x 日本語")
        self.ask("save", odd)
        self.ready(odd)
        self.move("parent", self.root)
        self.move("back", odd)

    def test_a_very_deep_folder_reads_back(self):
        deep = self.made(*["level%02d" % i for i in range(14)])
        self.ask("save", deep)
        self.assertTrue(self.folder_is(deep, 20), "the rails could not read a path of %d characters" % len(deep))

    def test_a_folder_reached_through_a_link_reads_back_as_the_link_or_its_target(self):
        target = self.made("real-place")
        link = os.path.join(self.root, "link-to-place")
        if not os.path.lexists(link):
            os.symlink(target, link)
        self.ask("save", link)
        got = wait_for(20, lambda: (self.folder_is(link, 2) and link) or (self.folder_is(target, 2) and target))
        self.assertTrue(got, "the rails read neither the link nor what it points to")

    def test_a_dialog_with_no_start_folder_reads_a_real_folder_or_says_unknown(self):
        self.ask("save", "-")
        time.sleep(3)
        for _ in range(3):
            self.s.run([OFOLDER, "do", "copy_path"])
            time.sleep(0.8)
        said = self.clipboard(1)
        self.assertTrue(
            said is None or os.path.isdir(said), "the rails named a folder that does not exist: %r" % said
        )


class FailOpen(unittest.TestCase):
    """What happens to the person's dialog when ofolder is missing, broken or stopped. The rule is
    that a dialog must still open and work: ofolder adds to a dialog and must never be what stops one.
    Run on Flea 0.4, the default chooser."""

    @classmethod
    def setUpClass(cls):
        require_build()
        if not shutil.which("flea"):
            raise unittest.SkipTest("flea is not installed")
        cls.host = Host()
        cls.session = Session(cls.host).__enter__()
        cls.root = tempfile.mkdtemp(prefix="ofe2e-data.")
        os.makedirs(os.path.join(cls.root, "one"))

    @classmethod
    def tearDownClass(cls):
        cls.session.stop()
        shutil.rmtree(cls.root, ignore_errors=True)

    def dialog_with(self, ofolder_exec, chain="ofolder;flea;gtk", guard=False, **stack):
        """A Save dialog asked for through a chain that starts with an ofolder run as `ofolder_exec`.
        Returns what the application heard, after the dialog is confirmed with Return."""
        s = self.session
        with Stack(s, chain, ofolder_exec=ofolder_exec, **stack) as stack:
            if guard:
                r = s.run(["sh", os.path.join(REPO, "packaging", "ofolder-guard.sh")],
                          env={"OFOLDER_GUARD_NO_RESTART": "1", "OFOLDER_GUARD_QUIET": "1"})
                self.assertEqual(r.returncode, 0, r.stderr)
            proc, out = stack.ask("save", os.path.join(self.root, "one"))
            try:
                cls = wait_for(40, lambda: next((c["class"] for c in s.clients() if c["class"] in ("com.thisisgm.flea.picker", "xdg-desktop-portal-gtk")), None))
                self.assertTrue(cls, "no dialog opened at all:\n" + s.log("app")[-400:] + "\n".join(l for l in s.log("bus").splitlines() if "ofolder" in l.lower() or l.startswith("Error"))[:1500])
                time.sleep(3)
                answer = None
                for _ in range(5):
                    s.press(cls, "Return")
                    answer = wait_for(2, lambda: os.path.exists(out) and open(out).read())
                    if answer:
                        break
                return cls, answer
            finally:
                proc.terminate()
                s.kill_session_processes()

    @unittest.expectedFailure
    def test_a_missing_ofolder_with_no_guard_still_gets_a_dialog(self):
        """KNOWN LIMIT: the portal asks the program that is not there, and every file dialog on the
        machine fails until it is put back or the portal file is removed. Nothing of ofolder's can run
        to prevent it, which is why the guard (below) exists and is a plain script."""
        cls, answer = self.dialog_with("/nonexistent/ofolder portal")
        self.assertTrue(answer and answer.startswith("SAVED "), "the application heard %r" % answer)

    def guard(self):
        r = self.session.run(["sh", os.path.join(REPO, "packaging", "ofolder-guard.sh")],
                             env={"OFOLDER_GUARD_NO_RESTART": "1", "OFOLDER_GUARD_QUIET": "1"})
        self.assertEqual(r.returncode, 0, r.stderr)
        return r

    def heard(self, stack):
        s = self.session
        proc, out = stack.ask("save", os.path.join(self.root, "one"))
        try:
            cls = wait_for(40, lambda: next((c["class"] for c in s.clients() if c["class"] in ("com.thisisgm.flea.picker", "xdg-desktop-portal-gtk")), None))
            self.assertTrue(cls, "no dialog opened at all:\n" + s.log("app")[-300:])
            time.sleep(3)
            answer = None
            for _ in range(5):
                s.press(cls, "Return")
                answer = wait_for(2, lambda: os.path.exists(out) and open(out).read())
                if answer:
                    break
            return answer
        finally:
            proc.terminate()
            s.kill_session_processes()

    def copy_of_ofolder(self):
        folder = os.path.join(self.session.dir, "installed")
        os.makedirs(folder, exist_ok=True)
        copy = os.path.join(folder, "ofolder")
        # A link, not a copy: the debug build is hundreds of megabytes and /tmp is a memory disk.
        os.symlink(OFOLDER, copy)
        return copy

    def test_ofolder_deleted_while_the_portal_runs_still_gets_a_dialog_once_the_guard_has_run(self):
        """How it happens: the program was fine when the portal started, then it is deleted."""
        copy = self.copy_of_ofolder()
        with Stack(self.session, "ofolder;flea;gtk", ofolder_exec=copy + " portal") as stack:
            os.remove(copy)
            self.guard()
            answer = self.heard(stack)
        self.assertTrue(answer and answer.startswith("SAVED "), "the application heard %r" % answer)

    def test_ofolder_missing_when_the_portal_starts_gets_a_dialog_once_the_guard_has_run_and_the_portal_restarts(self):
        """The login case: the guard restarts the portal after setting the file aside, which here is
        the private frontend being started again."""
        with Stack(self.session, "ofolder;flea;gtk", ofolder_exec="/nonexistent/ofolder portal") as stack:
            self.guard()
            stack.restart_frontend()
            answer = self.heard(stack)
        self.assertTrue(answer and answer.startswith("SAVED "), "the application heard %r" % answer)

    def test_an_ofolder_that_cannot_start_still_gets_a_dialog_once_the_guard_has_run(self):
        """A bad build: there and runnable but dead. The guard asks it for its version."""
        bad = os.path.join(self.session.dir, "bad-ofolder")
        open(bad, "w").write("#!/bin/sh\nexit 1\n")
        os.chmod(bad, 0o755)
        with Stack(self.session, "ofolder;flea;gtk", ofolder_exec=bad + " portal") as stack:
            self.guard()
            stack.restart_frontend()
            answer = self.heard(stack)
        self.assertTrue(answer and answer.startswith("SAVED "), "the application heard %r" % answer)

    @unittest.expectedFailure
    def test_an_ofolder_that_exits_at_once_with_no_guard_still_gets_a_dialog(self):
        """KNOWN LIMIT: without the guard, a program that is there but dead stops every dialog. With
        it, the program is asked for its version and set aside if it will not answer (above). What
        remains is a build that answers `version` and then fails inside the portal."""
        cls, answer = self.dialog_with("/usr/bin/false")
        self.assertTrue(answer and answer.startswith("SAVED "), "the application heard %r" % answer)

    def test_a_chain_that_names_a_backend_that_is_gone_still_gets_a_dialog(self):
        """The chain says ofolder then Strata then GTK, and Strata has been uninstalled."""
        cls, answer = self.dialog_with(None, chain="ofolder;strata;gtk", strata_installed=False)
        self.assertTrue(answer and answer.startswith("SAVED "), "the application heard %r" % answer)

    def test_stopping_the_rails_leaves_the_dialog_working(self):
        s = self.session
        with Stack(s, "ofolder;flea;gtk") as stack:
            proc, out = stack.ask("save", os.path.join(self.root, "one"))
            try:
                self.assertTrue(wait_for(40, lambda: all(s.client(c) for c in STRIPS)), "no rails")
                time.sleep(3)
                for pid in (int(d) for d in os.listdir("/proc") if d.isdigit()):
                    try:
                        cmd = open("/proc/%d/cmdline" % pid, "rb").read()
                        env = open("/proc/%d/environ" % pid, "rb").read()
                    except OSError:
                        continue
                    if b"ofolder\0strip" in cmd and ("XDG_RUNTIME_DIR=%s\0" % s.rt).encode() in env:
                        os.kill(pid, signal.SIGKILL)
                self.assertTrue(wait_for(10, lambda: not any(s.client(c) for c in STRIPS)), "the rails did not stop")
                self.assertTrue(s.client("com.thisisgm.flea.picker"), "the dialog went with the rails")
                answer = None
                for _ in range(5):
                    s.press("com.thisisgm.flea.picker", "Return")
                    answer = wait_for(2, lambda: os.path.exists(out) and open(out).read())
                    if answer:
                        break
                self.assertTrue(answer and answer.startswith("SAVED "), "the application heard %r" % answer)
            finally:
                proc.terminate()
                s.kill_session_processes()


class Recovery(unittest.TestCase):
    """Something outside ofolder takes it out of the file-chooser chain (as Flea once did): `doctor`
    must say so, `repair` must put it back and keep a copy, the rails must return, and `--undo` must
    restore the file as it was. Every call here passes --no-restart: the real session's portal service
    must never be restarted by a test."""

    @classmethod
    def setUpClass(cls):
        require_build()
        if not shutil.which("flea"):
            raise unittest.SkipTest("flea is not installed")
        cls.host = Host()
        cls.session = Session(cls.host).__enter__()
        cls.root = tempfile.mkdtemp(prefix="ofe2e-data.")
        os.makedirs(os.path.join(cls.root, "one"))

    @classmethod
    def tearDownClass(cls):
        cls.session.stop()
        shutil.rmtree(cls.root, ignore_errors=True)

    def ofolder(self, *args):
        return self.session.run([OFOLDER, *args], timeout=60)

    def test_a_chain_that_lost_ofolder_is_diagnosed_repaired_and_can_be_undone(self):
        s = self.session
        conf = os.path.join(s.env["HOME"], ".config/xdg-desktop-portal/hyprland-portals.conf")
        with Stack(s, "flea;gtk") as stack:  # the state Flea left behind
            original = open(conf).read()
            self.assertNotIn("ofolder", original)
            # Diagnosed.
            report = self.ofolder("doctor")
            self.assertEqual(report.returncode, 1, report.stdout)
            self.assertIn("PROBLEM", report.stdout)
            self.assertIn("not in the file chooser chain", report.stdout)
            # Without ofolder in the chain a dialog has no rails.
            proc, out = stack.ask("save", os.path.join(self.root, "one"))
            self.assertTrue(wait_for(40, lambda: s.client("com.thisisgm.flea.picker")))
            self.assertFalse(wait_for(5, lambda: s.client("ofolder.strip.left")), "rails with ofolder out of the chain")
            s.press("com.thisisgm.flea.picker", "Escape")
            proc.wait(20)
            # Repaired, with a copy kept.
            fixed = self.ofolder("repair", "--no-restart")
            self.assertEqual(fixed.returncode, 0, fixed.stdout + fixed.stderr)
            self.assertIn("kept a copy", fixed.stdout)
            self.assertIn("FileChooser=ofolder;flea;gtk", open(conf).read())
            backups = os.listdir(os.path.join(s.env["HOME"], ".local/state/ofolder/portal-backups"))
            self.assertEqual(len(backups), 1, backups)
            # Healthy, and the rails return once the frontend has re-read its configuration.
            self.assertEqual(self.ofolder("doctor").returncode, 0, self.ofolder("doctor").stdout)
            stack.restart_frontend()
            proc, out = stack.ask("save", os.path.join(self.root, "one"))
            self.assertTrue(wait_for(40, lambda: all(s.client(c) for c in STRIPS)), "no rails after the repair")
            s.press("com.thisisgm.flea.picker", "Escape")
            proc.wait(20)
            wait_for(10, lambda: not s.client("ofolder.strip.left"))
            # Repair again changes nothing; undo restores the file as it was.
            again = self.ofolder("repair", "--no-restart")
            self.assertIn("already starts with ofolder", again.stdout)
            self.assertEqual(len(os.listdir(os.path.join(s.env["HOME"], ".local/state/ofolder/portal-backups"))), 1)
            self.ofolder("repair", "--undo")
            self.assertEqual(open(conf).read(), original, "undo did not restore the file as it was")

    def test_repair_recreates_a_missing_registration(self):
        s = self.session
        service = os.path.join(s.env["HOME"], ".local/share/dbus-1/services/org.freedesktop.impl.portal.desktop.ofolder.service")
        portal_file = os.path.join(s.env["HOME"], ".local/share/xdg-desktop-portal/portals/ofolder.portal")
        with Stack(s, "ofolder;flea;gtk"):
            os.remove(service)
            os.remove(portal_file)
            report = self.ofolder("doctor")
            self.assertEqual(report.returncode, 1)
            self.assertIn("service file is missing", report.stdout)
            self.assertIn("portal file is missing", report.stdout)
            self.assertEqual(self.ofolder("repair", "--no-restart").returncode, 0)
            self.assertTrue(os.path.isfile(service) and os.path.isfile(portal_file))
            self.assertIn("Exec=", open(service).read())
            self.assertEqual(self.ofolder("doctor").returncode, 0, self.ofolder("doctor").stdout)


class Flea04(RealApps, Features, Rails, unittest.TestCase):
    name = "flea04"
    chain = "ofolder;flea;gtk"
    picker_class = "com.thisisgm.flea.picker"
    ipc = "flea"
    pick_marker = "--pick"

    def test_native_cancel_is_a_clean_no(self):
        """Flea 0.4 can be told to quit: the app hears the user's own no."""
        self.open_dialog()
        pid = self.picker_pid()
        r = self.s.run(["flea", "ipc", "--pid", str(pid), "call", "fleapicker", "quit"], timeout=6)
        self.assertEqual(r.returncode, 0, r.stderr)
        ans = self.answer()
        self.assertTrue(ans and ans.startswith("CANCELLED"), ans)
        self.proc = None


class Flea03(RealApps, Features, Rails, unittest.TestCase):
    settle_before_keys = 3.0
    folder_keys = ["Tab", "Tab", "Return"]
    save_keys = ["Return"]
    replace_keys = ["Tab", "Tab", "Return"]
    name = "flea03"
    chain = "ofolder;flea;gtk"
    picker_class = "com.thisisgm.flea.picker"
    ipc = "qs"
    pick_marker = "picker.qml"


class Gtk(RealApps, Features, Rails, unittest.TestCase):
    folder_keys = ["Return"]
    save_keys = ["Return"]
    replace_keys = ["Return"]
    name = "gtk"
    chain = "ofolder;gtk"
    picker_class = "xdg-desktop-portal-gtk"
    ipc = None
    pick_marker = ""


class Strata(RealApps, Features, Rails, unittest.TestCase):
    save_keys = ["Return"]
    replace_keys = ["Right", "Return"]
    folder_select_unknown = True  # Enter, Tab, Ctrl/Shift/Alt+Enter and a click were all tried
    multi_select_unknown = True  # Ctrl+A, Shift+Down and Space were tried
    filtered_needs_down = False
    name = "strata"
    chain = "ofolder;strata;gtk"
    picker_class = "io.github.lgse.Strata.FileChooser"
    ipc = None
    pick_marker = ""


if __name__ == "__main__":
    unittest.main(verbosity=2)
