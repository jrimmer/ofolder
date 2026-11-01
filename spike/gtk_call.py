# Calls the GTK portal backend's FileChooser.SaveFile directly (bypasses portals.conf), blocks until the dialog closes.
import sys, json
import gi
from gi.repository import Gio, GLib
folder = sys.argv[1].encode() + b"\0"
bus = Gio.bus_get_sync(Gio.BusType.SESSION)
opts = {"current_name": GLib.Variant("s", "spike.txt"), "current_folder": GLib.Variant("ay", folder)}
args = GLib.Variant("(osssa{sv})", ("/org/freedesktop/portal/desktop/request/spike/gtk1", "spike", "", "GTK spike save", opts))
r = bus.call_sync("org.freedesktop.impl.portal.desktop.gtk", "/org/freedesktop/portal/desktop",
                  "org.freedesktop.impl.portal.FileChooser", "SaveFile", args, None, Gio.DBusCallFlags.NONE, 90000)
print("REPLY", r.unpack(), flush=True)
