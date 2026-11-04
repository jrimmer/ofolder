"""A small GTK4 program that asks the portal for a Save or Open dialog and writes the answer to a file.

usage: app.py OUT save|open|multi|folder|filtered START_FOLDER [NAME]
OUT then holds `SAVED <path>`, `OPENED <path>` (several joined by `|`) or `CANCELLED <reason>`.
`filtered` is an Open dialog that offers only *.txt files. A START_FOLDER of `-` sets none.
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
        if start != "-":
            dialog.set_initial_folder(Gio.File.new_for_path(start))
        if mode == "filtered":
            text = Gtk.FileFilter()
            text.set_name("Text files")
            text.add_pattern("*.txt")
            filters = Gio.ListStore.new(Gtk.FileFilter)
            filters.append(text)
            dialog.set_filters(filters)
            dialog.set_default_filter(text)

        def done(d, res):
            try:
                if mode == "save":
                    text = "SAVED " + d.save_finish(res).get_path()
                elif mode == "multi":
                    files = d.open_multiple_finish(res)
                    text = "OPENED " + "|".join(files.get_item(i).get_path() for i in range(files.get_n_items()))
                elif mode == "folder":
                    text = "OPENED " + d.select_folder_finish(res).get_path()
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
        elif mode == "multi":
            dialog.open_multiple(win, None, done)
        elif mode == "folder":
            dialog.select_folder(win, None, done)
        else:
            dialog.open(win, None, done)
        return False

    GLib.timeout_add(500, ask)


app.connect("activate", on_activate)
app.run([])
