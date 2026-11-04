"""End to end, through the real portal path: a GTK4 program asks for a Save dialog, the real
xdg-desktop-portal frontend routes it to ofolder, which forwards to a real Flea (0.4 and 0.3) or GTK
dialog and docks its rails beside it. Run with `bin/e2e`.

Every test works in scratch folders inside a private nested Hyprland. Keys are typed only into a
window verified to be the dialog, and no real folder is read or written.
"""
import json
import os
import shutil
import subprocess
import sys
import tempfile
import time
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import flea03
from lib import OFOLDER, Host, Session, run, say, wait_for
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
            said = [l.strip() for l in open(os.path.join(self.s.dir, "bus.log")) if "ofolder strip:" in l][-30:]
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
        self.assertTrue(self.s.type_text(MENU, key="Return"))
        self.assertTrue(self.folder_is(self.two, 12), "find did not take the dialog to the match")

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


class Flea04(Features, Rails, unittest.TestCase):
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


class Flea03(Features, Rails, unittest.TestCase):
    name = "flea03"
    chain = "ofolder;flea;gtk"
    picker_class = "com.thisisgm.flea.picker"
    ipc = "qs"
    pick_marker = "picker.qml"


class Gtk(Features, Rails, unittest.TestCase):
    name = "gtk"
    chain = "ofolder;gtk"
    picker_class = "xdg-desktop-portal-gtk"
    ipc = None
    pick_marker = ""


class Strata(Features, Rails, unittest.TestCase):
    name = "strata"
    chain = "ofolder;strata;gtk"
    picker_class = "io.github.lgse.Strata.FileChooser"
    ipc = None
    pick_marker = ""


if __name__ == "__main__":
    unittest.main(verbosity=2)
