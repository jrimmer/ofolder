"""`ofolder uninstall` in a temporary home. No display, no systemd (OFOLDER_NO_SYSTEMD), no portal
restart (--no-restart), no real folder: every path is under a private HOME, and the program is a
link to the debug build, never copied."""
import hashlib
import os
import shutil
import subprocess
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(HERE)
TARGET = os.environ.get("CARGO_TARGET_DIR") or os.path.join(REPO, "target")
OFOLDER = os.environ.get("OFOLDER_BIN") or os.path.join(TARGET, "debug", "ofolder")
FAKE_PRAECO = os.path.join(REPO, "tests", "support", "fake-praeco")
CHOOSER = "org.freedesktop.impl.portal.FileChooser"
ORIGINAL_CONF = "[preferred]\n# my own note\ndefault=hyprland;gtk\n"


def sha(path):
    with open(path, "rb") as f:
        return hashlib.sha256(f.read()).hexdigest()


@unittest.skipUnless(os.path.exists(OFOLDER), "build first: cargo build --locked")
class Uninstall(unittest.TestCase):
    def setUp(self):
        self.home = tempfile.mkdtemp(prefix="ofe2e-uninstall.")
        self.addCleanup(shutil.rmtree, self.home, True)
        self.bin = os.path.join(self.home, ".local/bin")
        os.makedirs(self.bin)
        self.exe = os.path.join(self.bin, "ofolder")
        os.symlink(OFOLDER, self.exe)
        self.data = os.path.join(self.home, ".local/share")
        self.config = os.path.join(self.home, ".config")
        self.state = os.path.join(self.home, ".local/state")
        self.portals = os.path.join(self.data, "xdg-desktop-portal/portals")
        self.service = os.path.join(self.data, "dbus-1/services/org.freedesktop.impl.portal.desktop.ofolder.service")
        self.portal = os.path.join(self.portals, "ofolder.portal")
        self.conf = os.path.join(self.config, "xdg-desktop-portal/hyprland-portals.conf")
        self.settings = os.path.join(self.config, "ofolder/settings.toml")
        self.favorites = os.path.join(self.config, "ofolder/favorites.json")
        self.recents = os.path.join(self.data, "ofolder/recents.json")
        self.dialogs = os.path.join(self.data, "ofolder/dialogs.json")
        self.gtk_portal = os.path.join(self.portals, "gtk.portal")
        for path, text in (
            (self.conf, ORIGINAL_CONF),
            (self.gtk_portal, "[portal]\nDBusName=org.freedesktop.impl.portal.desktop.gtk\nInterfaces=%s;\n" % CHOOSER),
            (self.settings, "# kept\nrecents_max = 7\n"),
            (self.favorites, "[]\n"),
            (self.recents, "{}\n"),
            (self.dialogs, "{}\n"),
            (os.path.join(self.state, "ofolder/messages.log"), "a line\n"),
        ):
            os.makedirs(os.path.dirname(path), exist_ok=True)
            with open(path, "w") as f:
                f.write(text)
        self.env = {
            "HOME": self.home,
            "PATH": os.environ["PATH"],
            "XDG_CURRENT_DESKTOP": "Hyprland",
            "XDG_DATA_DIRS": os.path.join(self.home, "no-system-data"),
            "XDG_RUNTIME_DIR": os.path.join(self.home, "run"),
            "OFOLDER_NO_SYSTEMD": "1",
        }
        os.makedirs(self.env["XDG_RUNTIME_DIR"], mode=0o700)
        # The registration files repair would write are made by repair itself; the program is
        # given as the service file's program so no path outside HOME is written there.
        os.makedirs(os.path.dirname(self.service), exist_ok=True)
        with open(self.service, "w") as f:
            f.write("[D-BUS Service]\nName=org.freedesktop.impl.portal.desktop.ofolder\nExec=%s portal\n" % self.exe)

    def ofolder(self, *args):
        return subprocess.run([self.exe, *args], env=self.env, capture_output=True, text=True, timeout=60)

    def set_up(self):
        r = self.ofolder("repair", "--no-restart")
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        self.assertIn("%s=ofolder;gtk" % CHOOSER, open(self.conf).read())
        self.assertTrue(os.path.isfile(self.portal))
        self.guard_files = [
            os.path.join(self.data, "ofolder/ofolder-guard.sh"),
            os.path.join(self.config, "systemd/user/ofolder-guard.path"),
            os.path.join(self.config, "systemd/user/ofolder-guard.service"),
        ]
        for f in self.guard_files:
            self.assertTrue(os.path.isfile(f), f)

    def test_uninstall_undoes_the_setup_and_keeps_the_owners_data(self):
        self.set_up()
        r = self.ofolder("uninstall", "--no-restart")
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        self.assertEqual(open(self.conf).read(), ORIGINAL_CONF, "the chain is not as it was before the repair")
        for gone in (self.portal, self.service, *self.guard_files, self.dialogs, os.path.join(self.state, "ofolder")):
            self.assertFalse(os.path.exists(gone), "still there: " + gone)
        for kept in (self.settings, self.favorites, self.recents, self.gtk_portal, self.exe):
            self.assertTrue(os.path.exists(kept), "removed but should stay: " + kept)
        self.assertEqual(open(self.settings).read(), "# kept\nrecents_max = 7\n")
        self.assertIn("install.sh --uninstall", r.stdout)
        # The guard must not be able to put the chain back: nothing of it is left to run.
        self.assertFalse(os.path.exists(os.path.join(self.config, "systemd/user/ofolder-guard.path")))

    def test_keep_settings_is_the_default_and_can_be_said(self):
        self.set_up()
        r = self.ofolder("uninstall", "--keep-settings", "--no-restart")
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        self.assertTrue(os.path.isfile(self.settings))

    def test_purge_says_what_goes_and_removes_settings_favorites_and_recents(self):
        self.set_up()
        r = self.ofolder("uninstall", "--purge", "--no-restart")
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        for path in (self.settings, self.favorites, self.recents):
            self.assertIn(path, r.stdout, "the output did not name " + path)
            self.assertFalse(os.path.exists(path), "still there: " + path)
        said = r.stdout.index(self.settings)
        self.assertLess(said, r.stdout.index("removed " + self.settings), "it said what goes only after removing it")
        self.assertFalse(os.path.exists(os.path.join(self.config, "ofolder")), "an empty folder of ours was left")
        self.assertEqual(open(self.conf).read(), ORIGINAL_CONF)
        self.assertTrue(os.path.isfile(self.gtk_portal), "another program's portal file was removed")

    def test_a_chain_edited_after_the_repair_only_loses_ofolders_entry(self):
        self.set_up()
        with open(self.conf, "a") as f:
            f.write("org.freedesktop.impl.portal.Secret=gnome-keyring\n")
        later = os.stat(self.conf).st_mtime + 3600
        os.utime(self.conf, (later, later))
        r = self.ofolder("uninstall", "--no-restart")
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        text = open(self.conf).read()
        self.assertIn("org.freedesktop.impl.portal.Secret=gnome-keyring", text)
        self.assertIn("%s=gtk\n" % CHOOSER, text)
        self.assertNotIn("ofolder", text)

    def test_a_second_run_changes_nothing_and_succeeds(self):
        self.set_up()
        self.assertEqual(self.ofolder("uninstall", "--no-restart").returncode, 0)
        before = sha(self.conf)
        r = self.ofolder("uninstall", "--no-restart")
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        self.assertIn("nothing to undo", r.stdout)
        self.assertEqual(sha(self.conf), before)

    def test_a_machine_never_set_up_is_left_alone(self):
        os.remove(self.service)
        before = sha(self.conf)
        r = self.ofolder("uninstall", "--no-restart")
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        self.assertEqual(sha(self.conf), before)
        self.assertTrue(os.path.isfile(self.settings))

    def test_a_hold_left_by_an_undo_is_removed_with_the_rest_of_the_state(self):
        self.set_up()
        self.assertEqual(self.ofolder("repair", "--undo").returncode, 0)
        hold = os.path.join(self.state, "ofolder/chain-hold")
        self.assertTrue(os.path.isfile(hold))
        self.assertEqual(self.ofolder("uninstall", "--no-restart").returncode, 0)
        self.assertFalse(os.path.exists(hold))

    def test_the_registration_with_omarchy_update_is_taken_back(self):
        self.set_up()
        praeco_dir = os.path.join(self.home, "praeco-bin")
        os.makedirs(praeco_dir)
        os.symlink(FAKE_PRAECO, os.path.join(praeco_dir, "praeco"))
        self.env["OFOLDER_PRAECO"] = os.path.join(praeco_dir, "praeco")
        reg = os.path.join(self.config, "praeco/apps.d/ofolder.json")
        os.makedirs(os.path.dirname(reg))
        open(reg, "w").write("{}\n")
        r = self.ofolder("uninstall", "--no-restart")
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        calls = open(os.path.join(praeco_dir, "calls.log")).read().split("\n")
        self.assertIn("init", calls)
        self.assertIn("--remove", calls)

    def test_contradicting_or_unknown_options_are_refused_and_change_nothing(self):
        self.set_up()
        for args in (["--purge", "--keep-settings"], ["--everything"]):
            r = self.ofolder("uninstall", "--no-restart", *args)
            self.assertNotEqual(r.returncode, 0, args)
        self.assertTrue(os.path.isfile(self.portal))
        self.assertTrue(os.path.isfile(self.settings))


if __name__ == "__main__":
    unittest.main(verbosity=2)
