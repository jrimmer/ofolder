"""The guard script and `ofolder repair --watch`, in a temporary home. No display, no systemctl, no
restart of any portal: the guard is run with OFOLDER_GUARD_NO_RESTART and `--no-enable`. The session
bus is a private dbus-daemon that the test starts, because `portal --check` asks the bus for the
backend's name; the real desktop bus is never used."""
import os
import shutil
import subprocess
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(HERE)
TARGET = os.environ.get("CARGO_TARGET_DIR") or os.path.join(REPO, "target")
OFOLDER = os.environ.get("OFOLDER_BIN") or os.path.join(TARGET, "debug", "ofolder")
GUARD = os.path.join(REPO, "packaging", "ofolder-guard.sh")
CHOOSER = "org.freedesktop.impl.portal.FileChooser"
BACKEND = "gtk"
BACKEND_NAME = "org.freedesktop.impl.portal.desktop.%s" % BACKEND


@unittest.skipUnless(os.path.exists(OFOLDER), "build first: cargo build --locked")
class Guard(unittest.TestCase):
    def setUp(self):
        self.home = tempfile.mkdtemp(prefix="ofe2e-guard.")
        self.addCleanup(shutil.rmtree, self.home, True)
        self.bin = os.path.join(self.home, ".local/bin")
        os.makedirs(self.bin)
        self.exe = os.path.join(self.bin, "ofolder")
        # A link, not a copy: a debug build is hundreds of megabytes, and the temporary folder is small.
        os.symlink(OFOLDER, self.exe)
        self.data = os.path.join(self.home, ".local/share")
        self.service = os.path.join(self.data, "dbus-1/services/org.freedesktop.impl.portal.desktop.ofolder.service")
        self.portal = os.path.join(self.data, "xdg-desktop-portal/portals/ofolder.portal")
        self.conf = os.path.join(self.home, ".config/xdg-desktop-portal/hyprland-portals.conf")
        # The backend the chain names after ofolder is installed in the private data folder, and it
        # is the only one: the system's own portal files are not read, so the choice is the same on
        # every machine.
        backend_portal = os.path.join(self.data, "xdg-desktop-portal/portals/%s.portal" % BACKEND)
        backend_service = os.path.join(self.home, "bus-services/%s.service" % BACKEND_NAME)
        for path, text in (
            (self.service, "[D-BUS Service]\nName=org.freedesktop.impl.portal.desktop.ofolder\nExec=%s portal\n" % self.exe),
            (self.portal, "[portal]\nDBusName=org.freedesktop.impl.portal.desktop.ofolder\nInterfaces=%s;\n" % CHOOSER),
            (backend_portal, "[portal]\nDBusName=%s\nInterfaces=%s;\n" % (BACKEND_NAME, CHOOSER)),
            (backend_service, "[D-BUS Service]\nName=%s\nExec=/bin/true\n" % BACKEND_NAME),
            (self.conf, "[preferred]\n%s=ofolder;flea;gtk\n" % CHOOSER),
        ):
            os.makedirs(os.path.dirname(path), exist_ok=True)
            open(path, "w").write(text)
        self.env = {
            "HOME": self.home,
            "PATH": os.environ["PATH"],
            "XDG_CURRENT_DESKTOP": "Hyprland",
            "XDG_DATA_DIRS": os.path.join(self.home, "no-system-data"),
            "OFOLDER_GUARD_NO_RESTART": "1",
            "OFOLDER_GUARD_QUIET": "1",
            "OFOLDER_NO_SYSTEMD": "1",
        }
        self.start_private_bus(os.path.join(self.home, "bus-services"))

    def start_private_bus(self, service_dir):
        """A session bus of the test's own, with the backend's name activatable on it."""
        config = os.path.join(self.home, "bus.conf")
        with open(config, "w") as f:
            f.write(
                '<!DOCTYPE busconfig PUBLIC "-//freedesktop//DTD D-BUS Bus Configuration 1.0//EN" '
                '"http://www.freedesktop.org/standards/dbus/1.0/busconfig.dtd">\n'
                "<busconfig><type>session</type>"
                "<listen>unix:tmpdir=%s</listen>"
                "<servicedir>%s</servicedir>"
                '<policy context="default"><allow send_destination="*" eavesdrop="true"/>'
                '<allow eavesdrop="true"/><allow own="*"/></policy></busconfig>\n' % (self.home, service_dir)
            )
        bus = subprocess.Popen(
            ["dbus-daemon", "--config-file", config, "--nofork", "--print-address=1"],
            stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True,
        )
        self.addCleanup(self.stop_bus, bus)
        address = bus.stdout.readline().strip()
        if not address.startswith("unix:"):
            bus.kill()
            self.fail("the private session bus did not start: %s" % bus.stderr.read())
        self.env["DBUS_SESSION_BUS_ADDRESS"] = address

    @staticmethod
    def stop_bus(bus):
        bus.terminate()
        bus.wait(timeout=10)

    def stand_in(self, body):
        """Replaces the program with a shell script that answers as an ofolder build would."""
        os.remove(self.exe)
        with open(self.exe, "w") as f:
            f.write("#!/bin/sh\n" + body)
        os.chmod(self.exe, 0o755)

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
        os.symlink(OFOLDER, self.exe)
        r = self.guard()
        self.assertIn("ofolder is back", r.stdout)
        self.assertTrue(os.path.isfile(self.portal))
        self.assertFalse(os.path.exists(held))

    def test_the_right_file_is_repaired_even_when_the_guard_was_given_no_desktop_name(self):
        """Started by a service that was not given XDG_CURRENT_DESKTOP, the guard must still repair
        hyprland-portals.conf, the file the frontend reads, and not write a new portals.conf."""
        open(self.conf, "w").write("[preferred]\n%s=flea;gtk\n" % CHOOSER)
        del self.env["XDG_CURRENT_DESKTOP"]
        r = self.guard()
        self.assertIn("is back first", r.stdout, r.stdout + r.stderr)
        self.assertIn("%s=ofolder;flea;gtk" % CHOOSER, open(self.conf).read())
        self.assertFalse(
            os.path.exists(os.path.join(os.path.dirname(self.conf), "portals.conf")),
            "a stray portals.conf was written that the frontend never reads",
        )

    def test_a_build_that_starts_but_fails_its_portal_check_is_set_aside_and_returns_with_a_good_one(self):
        self.stand_in('case "$*" in\n  version) echo "ofolder 0.1.0" ;;\n  "portal --check") exit 3 ;;\n  *) exit 0 ;;\nesac\n')
        r = self.guard()
        self.assertIn("fails its portal check", r.stdout)
        self.assertFalse(os.path.exists(self.portal), "a build that fails its portal check must not stay registered")
        held = os.path.join(self.data, "ofolder/ofolder.portal.held")
        self.assertTrue(os.path.isfile(held))
        self.stand_in('case "$*" in\n  version) echo "ofolder 0.1.0" ;;\n  "portal --check") exit 0 ;;\n  *) exit 0 ;;\nesac\n')
        r = self.guard()
        self.assertIn("ofolder is back", r.stdout)
        self.assertTrue(os.path.isfile(self.portal))
        self.assertFalse(os.path.exists(held))

    def test_an_older_build_that_does_not_know_the_check_keeps_its_portal_file(self):
        """An older build exits 1 on `portal --check` (an unknown option). That says nothing about
        whether it works, so the guard must leave the portal file where it is, for any status but 3."""
        for status in (1, 2, 124):
            with self.subTest(status=status):
                self.stand_in('case "$*" in\n  version) echo "ofolder 0.1.0" ;;\n  "portal --check") exit %d ;;\n  *) exit 0 ;;\nesac\n' % status)
                r = self.guard()
                self.assertEqual(r.returncode, 0, r.stderr)
                self.assertNotIn("fails its portal check", r.stdout)
                self.assertTrue(os.path.isfile(self.portal), "status %d must not set the portal file aside" % status)
                self.assertFalse(os.path.exists(os.path.join(self.data, "ofolder/ofolder.portal.held")))

    def test_a_program_that_is_there_but_will_not_start_is_treated_as_gone(self):
        os.remove(self.exe)
        open(self.exe, "w").write("#!/bin/sh\nexit 1\n")  # a bad build: there, runnable, and dead
        os.chmod(self.exe, 0o755)
        r = self.guard()
        self.assertIn("will not start", r.stdout)
        self.assertFalse(os.path.exists(self.portal), "a dead program must not stay registered")
        os.remove(self.exe)
        os.symlink(OFOLDER, self.exe)
        self.assertIn("ofolder is back", self.guard().stdout)
        self.assertTrue(os.path.isfile(self.portal))

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
        settings = os.path.join(self.home, ".config/ofolder/settings.toml")
        self.assertIn("guard = true", open(settings).read(), "the command line did not record the setting")
        # The installed script does the same job.
        os.remove(self.exe)
        subprocess.run(["sh", script], env=self.env, capture_output=True, timeout=30)
        self.assertFalse(os.path.exists(self.portal))
        r = self.ofolder_missing_ok("repair", "--watch", "off", "--no-enable")
        self.assertTrue(os.path.isfile(self.portal), "off did not put the held portal file back")
        self.assertFalse(os.path.exists(script))
        self.assertFalse(os.path.exists(os.path.join(units, "ofolder-guard.path")))
        self.assertIn("guard = false", open(settings).read(), "turning it off did not record the setting")

    def ofolder_missing_ok(self, *args):
        """The program was removed for the test above; run the real one from elsewhere."""
        return subprocess.run([OFOLDER, *args], env=self.env, capture_output=True, text=True, timeout=30)


if __name__ == "__main__":
    unittest.main(verbosity=2)
