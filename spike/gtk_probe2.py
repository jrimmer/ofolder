#!/usr/bin/env python3
"""Probe 2: read the GTK dialog's current folder over AT-SPI and move it, by keys and by accessibility actions.

Reader: the dialog's path bar is a row of toggle buttons (deepest first, ending in 'File System Root'); the CHECKED one
is the current folder. The path is rebuilt from the names and accepted only if it exists on disk.
"""
import os, subprocess, sys, time

import gi
gi.require_version("Atspi", "2.0")
from gi.repository import Atspi

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import gtk_probe1 as p1

p1.OUT = sys.argv[1] if len(sys.argv) > 1 else os.path.join(p1.HERE, "runs", "gtk2")
os.makedirs(p1.OUT, exist_ok=True)
say, TITLE, HOME = p1.say, p1.TITLE, p1.HOME
Atspi.init()


def portal_app():
    d = Atspi.get_desktop(0)
    for i in range(d.get_child_count()):
        a = d.get_child_at_index(i)
        if a and (a.get_name() or "") == "xdg-desktop-portal-gtk":
            return a


def find(n, pred):
    if pred(n):
        return n
    for j in range(n.get_child_count()):
        r = find(n.get_child_at_index(j), pred)
        if r:
            return r


def find_all(n, pred, out=None):
    out = [] if out is None else out
    if pred(n):
        out.append(n)
    for j in range(n.get_child_count()):
        find_all(n.get_child_at_index(j), pred, out)
    return out


def choosers():
    app = portal_app()
    if not app:
        return []
    return [app.get_child_at_index(i) for i in range(app.get_child_count())
            if app.get_child_at_index(i).get_role_name() == "file chooser" and app.get_child_at_index(i).get_name() == TITLE]


def read_folder(ch):
    """None on any transient accessibility error: the path bar is rebuilt while the folder changes."""
    try:
        return _read_folder(ch)
    except Exception:
        return None


def _read_folder(ch):
    pb = find(ch, lambda n: n.get_name() == "PathBar Layer")
    if not pb:
        return None
    row = pb.get_child_at_index(0)
    btns = [row.get_child_at_index(j) for j in range(row.get_child_count())]
    btns = [b for b in btns if b.get_role_name() == "toggle button"]
    names = [b.get_name() for b in btns]
    checked = [i for i, b in enumerate(btns) if b.get_state_set().contains(Atspi.StateType.CHECKED)]
    if len(checked) != 1 or "File System Root" not in names[checked[0]:]:
        return None
    chain = names[checked[0]:names.index("File System Root", checked[0])]
    path = "/" + "/".join(reversed(chain))
    return path if os.path.isdir(path) else None


def wait_read(ch, want, timeout=3.0):
    t0 = time.time()
    while time.time() - t0 < timeout:
        if read_folder(ch) == want:
            return time.time() - t0
        time.sleep(0.01)
    return None


def do_action(node, idx=0):
    return Atspi.Action.do_action(node, idx)


def actions(node):
    try:
        return [Atspi.Action.get_action_name(node, i) for i in range(Atspi.Action.get_n_actions(node))]
    except Exception:
        return []


def active_is(addr):
    return (p1.hj("activewindow") or {}).get("address") == addr


def type_path(d, text):
    """Select all in the Name entry, type the folder path, Return. Only into the verified-active dialog."""
    subprocess.run(["hyprctl", "dispatch", 'hl.dsp.focus({ window = "address:%s" })' % d["address"]], capture_output=True)
    time.sleep(0.3)
    if not active_is(d["address"]):
        say("REFUSED to type: dialog not active")
        return False
    subprocess.run(["wtype", "-M", "ctrl", "a", "-m", "ctrl"]); time.sleep(0.15)
    if not active_is(d["address"]):
        say("REFUSED to continue typing: focus moved"); return False
    subprocess.run(["wtype", "--", text]); time.sleep(0.2)
    subprocess.run(["wtype", "-k", "Return"])
    return True


def main():
    # 0. clean up stray dialogs from probe 1 without any keystrokes: press their Cancel via accessibility
    for ch in choosers():
        cancel = find(ch, lambda n: n.get_role_name() == "button" and n.get_name() == "Cancel")
        if cancel:
            say("closing stray dialog via Cancel action ->", do_action(cancel))
    time.sleep(1.0)
    call = subprocess.Popen([sys.executable, os.path.join(p1.HERE, "gtk_call.py"), HOME],
                            stdout=open(os.path.join(p1.OUT, "call.out"), "w"), stderr=subprocess.STDOUT)
    d = None
    for _ in range(100):
        d = p1.dialog()
        if d:
            break
        time.sleep(0.05)
    if not d:
        say("NO DIALOG"); return
    time.sleep(1.0)
    d = p1.dialog()
    chs = choosers()
    say("a11y choosers with our title:", len(chs), "| window pid", d["pid"], "class", d["class"])
    ch = chs[0]
    t0 = time.time(); r = read_folder(ch); say("READ at start: %s (%.0fms)" % (r, (time.time() - t0) * 1000))

    cases = [("typed Documents", HOME + "/Documents"), ("typed Work", HOME + "/Work"),
             ("typed space+unicode", os.path.join(HOME, "Work/ofolder/spike/runs/h3/playground/ünïcödé 日本語")),
             ("typed Downloads", HOME + "/Downloads")]
    for label, path in cases:
        if not os.path.isdir(path):
            say("SKIP", label, "(missing)"); continue
        if not type_path(d, path + "/"):
            continue
        lat = wait_read(ch, path)
        say("%s: %s latency=%s reading=%s" % (label, "PASS" if lat is not None else "FAIL", "%.0fms" % (lat * 1000) if lat is not None else "NONE", read_folder(ch)))
        time.sleep(0.2)

    # Accessibility actions: no keystrokes at all. Click an ancestor in the path bar, then a sidebar place.
    pb = find(ch, lambda n: n.get_name() == "PathBar Layer")
    row = pb.get_child_at_index(0)
    anc = next((row.get_child_at_index(j) for j in range(row.get_child_count()) if row.get_child_at_index(j).get_name() == os.path.basename(HOME)), None)
    if anc:
        say("path-bar button actions:", actions(anc))
        do_action(anc)
        lat = wait_read(ch, HOME)
        say("action: click path-bar '%s': %s latency=%s" % (os.path.basename(HOME), "PASS" if lat is not None else "FAIL", "%.0fms" % (lat * 1000) if lat is not None else "NONE"))
    places = find(ch, lambda n: n.get_name() == "Places")
    items = find_all(places, lambda n: n.get_role_name() == "list item")
    say("places list items:", len(items), "first item actions:", actions(items[0]) if items else None)
    labelled = []
    for it in items:
        lab = find(it, lambda n: n.get_role_name() == "label" and n.get_name())
        labelled.append((lab.get_name() if lab else None, it))
    say("places labels:", [l for l, _ in labelled])
    dl = next((it for l, it in labelled if l == "Downloads"), None)
    if dl:
        do_action(dl)
        lat = wait_read(ch, HOME + "/Downloads")
        say("action: click place 'Downloads': %s latency=%s" % ("PASS" if lat is not None else "FAIL", "%.0fms" % (lat * 1000) if lat is not None else "NONE"))
    subprocess.run(["grim", "-g", "%d,%d %dx%d" % (d["at"][0], d["at"][1], d["size"][0], d["size"][1]), os.path.join(p1.OUT, "end-state.png")])

    # Create folder button (GTK's own) is present; and close via Cancel action, not keys.
    cf = find(ch, lambda n: n.get_name() == "Create Folder")
    say("Create Folder button actions:", actions(cf) if cf else None)
    cancel = find(ch, lambda n: n.get_role_name() == "button" and n.get_name() == "Cancel")
    say("closing via Cancel action ->", do_action(cancel))
    try:
        call.wait(5)
    except subprocess.TimeoutExpired:
        say("call still waiting; killing"); call.kill()
    say("call output:", open(os.path.join(p1.OUT, "call.out")).read().strip()[:200])
    say("dialog windows left:", len([c for c in (p1.hj("clients") or []) if c["title"] == TITLE]))


if __name__ == "__main__":
    main()
