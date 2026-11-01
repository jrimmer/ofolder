"""Read-only: what app_id does the portal frontend pass to the file-chooser backend? Uses the DEFAULT configuration; changes nothing."""
import os, re, subprocess, sys, time, tempfile
os.environ.setdefault("HYPRLAND_INSTANCE_SIGNATURE", "efb50993780079460b0cbed1363e2166a2de1d9f_1792084656_1809977280")
os.environ.setdefault("WAYLAND_DISPLAY", "wayland-1")
CLIENT = r'''
import sys, gi
from gi.repository import Gio, GLib
bus = Gio.bus_get_sync(Gio.BusType.SESSION)
opts = {"current_name": GLib.Variant("s", "probe.txt")}
bus.call_sync("org.freedesktop.portal.Desktop", "/org/freedesktop/portal/desktop", "org.freedesktop.portal.FileChooser", "SaveFile",
              GLib.Variant("(ssa{sv})", ("", sys.argv[1], opts)), None, Gio.DBusCallFlags.NONE, 10000)
'''
client = tempfile.NamedTemporaryFile("w", suffix=".py", delete=False); client.write(CLIENT); client.close()
def run_variant(label, cmd):
    mon_out = tempfile.NamedTemporaryFile("w+", delete=False)
    mon = subprocess.Popen(["dbus-monitor", "--session", "type='method_call',interface='org.freedesktop.impl.portal.FileChooser',member='SaveFile'"], stdout=mon_out, stderr=subprocess.DEVNULL)
    time.sleep(0.8)
    title = "appid probe " + label
    subprocess.run(cmd + [sys.executable, client.name, title], timeout=30)
    time.sleep(1.5)
    mon.terminate(); mon.wait(3)
    text = open(mon_out.name).read(); os.unlink(mon_out.name)
    i = text.find("member=SaveFile")
    body = text[i:text.find("array [", i)] if i >= 0 else ""
    strings = re.findall(r'^\s+string "([^"]*)"', body, re.M)   # app_id, parent_window, title, in the call's order
    appid = strings[0] if strings else None
    print("%-34s forwarded app_id = %r  (title seen: %r)" % (label, appid, strings[2] if len(strings) > 2 else None), flush=True)
    # close any picker window left open
    time.sleep(0.5)
    r = subprocess.run(["hyprctl", "clients", "-j"], capture_output=True, text=True)
    import json
    for c in json.loads(r.stdout or "[]"):
        if c["title"] == title or (c["class"] == "com.thisisgm.flea.picker" and "appid probe" in c["title"]):
            subprocess.run(["hyprctl", "dispatch", 'hl.dsp.focus({ window = "address:%s" })' % c["address"]], capture_output=True); time.sleep(0.3)
            if json.loads(subprocess.run(["hyprctl", "activewindow", "-j"], capture_output=True, text=True).stdout or "{}").get("address") == c["address"]:
                subprocess.run(["wtype", "-k", "Escape"])
            time.sleep(0.5)
pid = os.getpid()
run_variant("plain process, no scope", [])
run_variant("scope app-Hyprland-<name>", ["systemd-run", "--user", "--scope", "--quiet", "--collect", "--unit=app-Hyprland-ofolderprobe-%d.scope" % pid, "--"])
run_variant("scope app-<desktop.id>", ["systemd-run", "--user", "--scope", "--quiet", "--collect", "--unit=app-org.example.Probe-%d.scope" % pid, "--"])
run_variant("scope with a REAL installed app id", ["systemd-run", "--user", "--scope", "--quiet", "--collect", "--unit=app-org.chromium.Chromium-%d.scope" % pid, "--"])
os.unlink(client.name)
