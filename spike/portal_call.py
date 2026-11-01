# Backend-agnostic: call <backend>'s org.freedesktop.impl.portal.FileChooser.SaveFile directly (bypasses portals.conf).
# usage: portal_call.py <backend: gtk|strata|flea> <folder> <title>
import sys
import gi
from gi.repository import Gio, GLib
backend, folder, title = sys.argv[1], sys.argv[2].encode() + b"\0", sys.argv[3]
bus = Gio.bus_get_sync(Gio.BusType.SESSION)
opts = {"current_name": GLib.Variant("s", "spike.txt"), "current_folder": GLib.Variant("ay", folder)}
args = GLib.Variant("(osssa{sv})", ("/org/freedesktop/portal/desktop/request/spike/%s1" % backend, "spike", "", title, opts))
r = bus.call_sync("org.freedesktop.impl.portal.desktop." + backend, "/org/freedesktop/portal/desktop",
                  "org.freedesktop.impl.portal.FileChooser", "SaveFile", args, None, Gio.DBusCallFlags.NONE, 90000)
print("REPLY", r.unpack(), flush=True)
