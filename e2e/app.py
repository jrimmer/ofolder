"""A small GTK4 program that asks the portal for a Save or Open dialog and writes the answer to a file.

usage: app.py OUT save|open START_FOLDER [NAME]
OUT then holds `SAVED <path>`, `OPENED <path>` or `CANCELLED <reason>`.
It has its own window class (org.ofolder.TestApp) and sends no portal app id.
"""
import os
import sys

import gi

gi.require_version("Gtk", "4.0")
from gi.repository import Gio, GLib, Gtk

out, mode, start = sys.argv[1], sys.argv[2], sys.argv[3]
name = sys.argv[4] if len(sys.argv) > 4 else "e2e.txt"
app = Gtk.Application(application_id="org.ofolder.TestApp")


def on_activate(a):
    win = Gtk.ApplicationWindow(application=a, title="ofolder test app")
    win.set_default_size(300, 120)
    win.present()

    def ask():
        dialog = Gtk.FileDialog()
        dialog.set_initial_folder(Gio.File.new_for_path(start))

        def done(d, res):
            try:
                if mode == "save":
                    text = "SAVED " + d.save_finish(res).get_path()
                else:
                    text = "OPENED " + d.open_finish(res).get_path()
            except GLib.Error as e:
                text = "CANCELLED " + str(e.message)
            with open(out + ".tmp", "w") as f:
                f.write(text)
            os.replace(out + ".tmp", out)
            a.quit()

        if mode == "save":
            dialog.set_initial_name(name)
            dialog.save(win, None, done)
        else:
            dialog.open(win, None, done)
        return False

    GLib.timeout_add(500, ask)


app.connect("activate", on_activate)
app.run([])
