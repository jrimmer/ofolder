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


class Flea04(Rails, unittest.TestCase):
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


class Flea03(Rails, unittest.TestCase):
    name = "flea03"
    chain = "ofolder;flea;gtk"
    picker_class = "com.thisisgm.flea.picker"
    ipc = "qs"
    pick_marker = "picker.qml"


class Gtk(Rails, unittest.TestCase):
    name = "gtk"
    chain = "ofolder;gtk"
    picker_class = "xdg-desktop-portal-gtk"
    ipc = None
    pick_marker = ""


if __name__ == "__main__":
    unittest.main(verbosity=2)
