"""A private desktop-portal stack inside a nested session: its own D-Bus, the real
xdg-desktop-portal frontend, ofolder's backend and the real Flea or GTK backend behind it.

The portal files and the configuration live in the session's private home, so the real
desktop's portal chain is neither read nor written.
"""
import os
import subprocess

from lib import OFOLDER, run, say, wait_for

FRONTEND = "/usr/lib/xdg-desktop-portal"
HERE = os.path.dirname(os.path.abspath(__file__))


class Stack:
    def __init__(self, session, chain, flea=None):
        """`chain` is the FileChooser line, e.g. "ofolder;flea;gtk". `flea` is an unpacked
        Flea release to use instead of the installed one (a dict of FLEA_BIN, FLEA_UI)."""
        self.s = session
        self.chain = chain
        self.flea = flea or {}
        self.bus = None
        self.frontend = None

    def __enter__(self):
        s = self.s
        home = s.env["HOME"]
        cfg = os.path.join(home, ".config/xdg-desktop-portal")
        portals = os.path.join(home, ".local/share/xdg-desktop-portal/portals")
        services = os.path.join(home, ".local/share/dbus-1/services")
        for d in (cfg, portals, services):
            os.makedirs(d, exist_ok=True)
        with open(os.path.join(cfg, "hyprland-portals.conf"), "w") as f:
            f.write(
                "[preferred]\ndefault=hyprland;gtk\n"
                "org.freedesktop.impl.portal.FileChooser=%s\n" % self.chain
            )
        with open(os.path.join(portals, "ofolder.portal"), "w") as f:
            f.write(
                "[portal]\nDBusName=org.freedesktop.impl.portal.desktop.ofolder\n"
                "Interfaces=org.freedesktop.impl.portal.FileChooser;\nUseIn=Hyprland\n"
            )
        # The compositor's own portal backend (screenshots, screen casts) is nothing this suite
        # uses, and it crashes inside a nested session, leaving a core dump for each run. A
        # same-named file in the private data directory is read first, so the frontend finds no
        # backend by that name to start.
        with open(os.path.join(portals, "hyprland.portal"), "w") as f:
            f.write(
                "[portal]\nDBusName=org.ofolder.e2e.NoSuchBackend\n"
                "Interfaces=org.freedesktop.impl.portal.Screenshot;\nUseIn=none\n"
            )
        with open(os.path.join(services, "org.freedesktop.impl.portal.desktop.ofolder.service"), "w") as f:
            f.write(
                "[D-BUS Service]\nName=org.freedesktop.impl.portal.desktop.ofolder\n"
                "Exec=%s portal\n" % OFOLDER
            )
        env = dict(s.env, **self.flea, OFOLDER_DEBUG="1")
        # A fresh address for every stack: a previous one's socket file can outlive its daemon.
        Stack.count = getattr(Stack, "count", 0) + 1
        self.path = "%s/bus-%d" % (s.rt, Stack.count)
        addr = "unix:path=" + self.path
        self.bus = subprocess.Popen(
            ["dbus-daemon", "--session", "--nofork", "--address=" + addr],
            env=env,
            stdout=self._log("bus"),
            stderr=subprocess.STDOUT,
            start_new_session=True,
        )
        s.env["DBUS_SESSION_BUS_ADDRESS"] = addr
        env["DBUS_SESSION_BUS_ADDRESS"] = addr
        if not wait_for(
            5, lambda: os.path.exists(self.path) and run(["busctl", "--user", "list", "--no-pager"], env=env).returncode == 0
        ):
            raise RuntimeError("the private D-Bus did not start")
        self.frontend = subprocess.Popen(
            [FRONTEND, "-v"],
            env=env,
            stdout=self._log("frontend"),
            stderr=subprocess.STDOUT,
            start_new_session=True,
        )
        ready = wait_for(
            40,
            lambda: run(
                ["busctl", "--user", "status", "org.freedesktop.portal.Desktop"], env=env
            ).returncode
            == 0,
        )
        if not ready:
            raise RuntimeError("the portal frontend did not come up:\n" + s.log("frontend"))
        return self

    def __exit__(self, *exc):
        for p in (self.frontend, self.bus):
            if p and p.poll() is None:
                p.terminate()
        for p in (self.frontend, self.bus):
            if p:
                try:
                    p.wait(5)
                except subprocess.TimeoutExpired:
                    p.kill()
        return False

    def _log(self, name):
        return open(os.path.join(self.s.dir, name + ".log"), "w")

    def ask(self, mode, start, name="e2e.txt"):
        """Start the test app, which opens a dialog through the portal. Returns (process, out path)."""
        out = os.path.join(self.s.dir, "answer-%d.txt" % len(self.s.children))
        p = self.s.spawn(
            ["python3", os.path.join(HERE, "app.py"), out, mode, start, name],
            env={"GTK_USE_PORTAL": "1"},
            name="app",
        )
        return p, out
