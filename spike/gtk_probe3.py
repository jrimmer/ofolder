#!/usr/bin/env python3
"""Probe 3: which ways of moving the GTK save dialog to an arbitrary folder actually work? Each variant starts from HOME."""
import os, subprocess, sys, time

import gi
gi.require_version("Atspi", "2.0")
from gi.repository import Atspi

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
sys.argv = [sys.argv[0], os.path.join(os.path.dirname(os.path.abspath(__file__)), "runs", "gtk3")]
import gtk_probe2 as p2
p1, say, HOME = p2.p1, p2.say, p2.HOME
TARGET = HOME + "/Documents"


def keys(d, *steps):
    """Each step is a wtype arg list; typed only while the dialog is verified active."""
    subprocess.run(["hyprctl", "dispatch", 'hl.dsp.focus({ window = "address:%s" })' % d["address"]], capture_output=True)
    time.sleep(0.3)
    for s in steps:
        if not p2.active_is(d["address"]):
            say("   REFUSED: dialog not active"); return False
        subprocess.run(["wtype", *s]); time.sleep(0.2)
    return True


def home(ch):
    pb = p2.find(ch, lambda n: n.get_name() == "PathBar Layer")
    row = pb.get_child_at_index(0)
    b = next((row.get_child_at_index(j) for j in range(row.get_child_count()) if row.get_child_at_index(j).get_name() == os.path.basename(HOME)), None)
    if b:
        p2.do_action(b)
    return p2.wait_read(ch, HOME, 2.0) is not None


def name_entry(ch):
    # the Name entry sits next to the label 'Name' in the header bar
    for t in p2.find_all(ch, lambda n: n.get_role_name() == "text"):
        if Atspi.Text.get_character_count(t) >= 0 and t.get_state_set().contains(Atspi.StateType.EDITABLE):
            return t


def result(label, ch, d):
    lat = p2.wait_read(ch, TARGET, 1.5)
    say("VARIANT %-34s %s %s" % (label, "WORKS" if lat is not None else "no", "latency=%.0fms" % (lat * 1000) if lat is not None else "reading=%s" % p2.read_folder(ch)))
    ok = lat is not None
    home(ch)
    return ok


def main():
    call = subprocess.Popen([sys.executable, os.path.join(p1.HERE, "gtk_call.py"), HOME], stdout=open(os.path.join(p1.OUT, "call.out"), "w"), stderr=subprocess.STDOUT)
    d = None
    for _ in range(100):
        d = p1.dialog()
        if d: break
        time.sleep(0.05)
    time.sleep(1.0)
    d = p1.dialog(); ch = p2.choosers()[0]
    say("start reading:", p2.read_folder(ch))
    ent = name_entry(ch)
    say("name entry found:", bool(ent), "| its actions:", p2.actions(ent) if ent else None)

    # V1: select all, type path/, Return, Return
    keys(d, ["-M", "ctrl", "a", "-m", "ctrl"], ["--", TARGET + "/"], ["-k", "Return"], ["-k", "Return"]); result("V1 ctrl+a, path/, Return x2", ch, d)
    # V2: path with no trailing slash
    keys(d, ["-M", "ctrl", "a", "-m", "ctrl"], ["--", TARGET], ["-k", "Return"]); result("V2 ctrl+a, path, Return", ch, d)
    # V3: Ctrl+L, path, Return
    keys(d, ["-M", "ctrl", "l", "-m", "ctrl"], ["--", TARGET + "/"], ["-k", "Return"]); result("V3 ctrl+L, path/, Return", ch, d)
    # V4: set the entry text through the accessibility EditableText interface, then Return key
    if ent:
        Atspi.EditableText.set_text_contents(ent, TARGET + "/")
        time.sleep(0.2)
        keys(d, ["-k", "Return"]); result("V4 atspi set_text, Return", ch, d)
    # V5: sidebar place via the Selection interface
    places = p2.find(ch, lambda n: n.get_name() == "Places")
    lst = places.get_child_at_index(0) if places else None
    items = p2.find_all(places, lambda n: n.get_role_name() == "list item")
    idx = next((i for i, it in enumerate(items) if (p2.find(it, lambda n: n.get_role_name() == "label" and n.get_name() == "Documents"))), None)
    say("   places container role:", lst and lst.get_role_name(), "documents idx:", idx)
    if idx is not None:
        try:
            ok = Atspi.Selection.select_child(places.get_child_at_index(0) if lst else places, idx)
            say("   select_child ->", ok)
        except Exception as e:
            say("   select_child error:", e)
        result("V5 atspi Selection.select_child", ch, d)
        # V6: grab focus on the place item, then Return
        items[idx].get_component_iface() if hasattr(items[idx], "get_component_iface") else None
        try:
            Atspi.Component.grab_focus(items[idx])
        except Exception as e:
            say("   grab_focus error:", e)
        keys(d, ["-k", "Return"]); result("V6 grab_focus place + Return", ch, d)
    # V7: type-ahead from the file list: focus list, type '/', path, Return
    tbl = p2.find(ch, lambda n: n.get_role_name() == "table" and n.get_name() == "Files")
    if tbl:
        try:
            Atspi.Component.grab_focus(tbl)
        except Exception as e:
            say("   table grab_focus error:", e)
        keys(d, ["--", TARGET + "/"], ["-k", "Return"]); result("V7 list focus, type path/, Return", ch, d)

    subprocess.run(["grim", "-g", "%d,%d %dx%d" % (d["at"][0], d["at"][1], d["size"][0], d["size"][1]), os.path.join(p1.OUT, "end-state.png")])
    cancel = p2.find(ch, lambda n: n.get_role_name() == "button" and n.get_name() == "Cancel")
    say("closing via Cancel ->", p2.do_action(cancel))
    try: call.wait(5)
    except subprocess.TimeoutExpired: call.kill()
    say("dialog windows left:", len([c for c in (p1.hj("clients") or []) if c["title"] == p1.TITLE]))


if __name__ == "__main__":
    os.makedirs(p1.OUT, exist_ok=True)
    main()
