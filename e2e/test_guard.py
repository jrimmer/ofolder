"""The guard script and `ofolder repair --watch`, in a temporary home. No display, no systemctl, no
restart of any portal: the guard is run with OFOLDER_GUARD_NO_RESTART and `--no-enable`."""
import os
import shutil
import subprocess
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(HERE)
OFOLDER = os.environ.get("OFOLDER_BIN") or os.path.join(REPO, "target", "debug", "ofolder")
GUARD = os.path.join(REPO, "packaging", "ofolder-guard.sh")
CHOOSER = "org.freedesktop.impl.portal.FileChooser"


@unittest.skipUnless(os.path.exists(OFOLDER), "build first: cargo build --locked")
class Guard(unittest.TestCase):
    def setUp(self):
        self.home = tempfile.mkdtemp(prefix="ofe2e-guard.")
        self.addCleanup(shutil.rmtree, self.home, True)
        self.bin = os.path.join(self.home, ".local/bin")
        os.makedirs(self.bin)
        self.exe = os.path.join(self.bin, "ofolder")
        shutil.copy(OFOLDER, self.exe)
        self.data = os.path.join(self.home, ".local/share")
        self.service = os.path.join(self.data, "dbus-1/services/org.freedesktop.impl.portal.desktop.ofolder.service")
        self.portal = os.path.join(self.data, "xdg-desktop-portal/portals/ofolder.portal")
        self.conf = os.path.join(self.home, ".config/xdg-desktop-portal/hyprland-portals.conf")
        for path, text in (
            (self.service, "[D-BUS Service]\nName=org.freedesktop.impl.portal.desktop.ofolder\nExec=%s portal\n" % self.exe),
            (self.portal, "[portal]\nDBusName=org.freedesktop.impl.portal.desktop.ofolder\nInterfaces=%s;\n" % CHOOSER),
            (self.conf, "[preferred]\n%s=ofolder;flea;gtk\n" % CHOOSER),
        ):
            os.makedirs(os.path.dirname(path), exist_ok=True)
            open(path, "w").write(text)
        self.env = {
            "HOME": self.home,
            "PATH": os.environ["PATH"],
            "XDG_CURRENT_DESKTOP": "Hyprland",
            "OFOLDER_GUARD_NO_RESTART": "1",
            "OFOLDER_GUARD_QUIET": "1",
        }

    def guard(self):
        return subprocess.run(["sh", GUARD], env=self.env, capture_output=True, text=True, timeout=30)

    def ofolder(self, *args):
        return subprocess.run([self.exe, *args], env=self.env, capture_output=True, text=True, timeout=30)

    def test_a_healthy_setup_is_left_alone(self):
        before = open(self.conf).read()
        r = self.guard()
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertTrue(os.path.isfile(self.portal))
        self.assertEqual(open(self.conf).read(), before)

    def test_a_missing_program_has_its_portal_file_set_aside_and_it_returns_with_the_program(self):
        os.remove(self.exe)
        r = self.guard()
        self.assertIn("ofolder is missing", r.stdout)
        self.assertFalse(os.path.exists(self.portal), "the portal would still ask for a program that is gone")
        held = os.path.join(self.data, "ofolder/ofolder.portal.held")
        self.assertTrue(os.path.isfile(held))
        self.guard()  # idempotent
        self.assertTrue(os.path.isfile(held))
        shutil.copy(OFOLDER, self.exe)
        r = self.guard()
        self.assertIn("ofolder is back", r.stdout)
        self.assertTrue(os.path.isfile(self.portal))
        self.assertFalse(os.path.exists(held))

    def test_a_chain_that_lost_ofolder_is_repaired_by_the_guard(self):
        open(self.conf, "w").write("[preferred]\n%s=flea;gtk\n" % CHOOSER)
        r = self.guard()
        self.assertIn("is back first", r.stdout)
        self.assertIn("%s=ofolder;flea;gtk" % CHOOSER, open(self.conf).read())
        backups = os.listdir(os.path.join(self.home, ".local/state/ofolder/portal-backups"))
        self.assertEqual(len(backups), 1)

    def test_watch_on_writes_the_guard_and_its_units_and_off_removes_them_and_restores_a_held_portal_file(self):
        r = self.ofolder("repair", "--watch", "on", "--no-enable")
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        script = os.path.join(self.data, "ofolder/ofolder-guard.sh")
        units = os.path.join(self.home, ".config/systemd/user")
        self.assertTrue(os.access(script, os.X_OK))
        path_unit = open(os.path.join(units, "ofolder-guard.path")).read()
        self.assertIn("PathChanged=%s\n" % self.bin, path_unit)
        self.assertIn("PathChanged=%s\n" % os.path.dirname(self.conf), path_unit)
        self.assertIn("ExecStart=%s\n" % script, open(os.path.join(units, "ofolder-guard.service")).read())
        self.assertEqual(open(script).read(), open(GUARD).read(), "the installed guard is not the packaged one")
        # The installed script does the same job.
        os.remove(self.exe)
        subprocess.run(["sh", script], env=self.env, capture_output=True, timeout=30)
        self.assertFalse(os.path.exists(self.portal))
        r = self.ofolder_missing_ok("repair", "--watch", "off", "--no-enable")
        self.assertTrue(os.path.isfile(self.portal), "off did not put the held portal file back")
        self.assertFalse(os.path.exists(script))
        self.assertFalse(os.path.exists(os.path.join(units, "ofolder-guard.path")))

    def ofolder_missing_ok(self, *args):
        """The program was removed for the test above; run the real one from elsewhere."""
        return subprocess.run([OFOLDER, *args], env=self.env, capture_output=True, text=True, timeout=30)


if __name__ == "__main__":
    unittest.main(verbosity=2)
