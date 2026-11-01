#!/usr/bin/env python3
"""Probe 5: move the GTK save dialog with ZERO keystrokes: AT-SPI set_text_contents on the Name entry, then its 'activate' action."""
import os, subprocess, sys, time
import gi
gi.require_version("Atspi", "2.0")
from gi.repository import Atspi
sys.argv = [sys.argv[0], os.path.join(os.path.dirname(os.path.abspath(__file__)), "runs", "gtk5")]
import gtk_probe2 as p2
p1, say, HOME = p2.p1, p2.say, p2.HOME
os.makedirs(p1.OUT, exist_ok=True)

def entry(ch):
    return next((t for t in p2.find_all(ch, lambda n: n.get_role_name() == "text") if t.get_state_set().contains(Atspi.StateType.EDITABLE)), None)

def goto(ch, path):
    e = entry(ch)
    Atspi.EditableText.set_text_contents(e, path.rstrip("/") + "/")
    ok = Atspi.Action.do_action(e, 0)
    lat = p2.wait_read(ch, path, 2.0)
    return ok, lat

for c in p2.choosers():                       # clean any stray dialog, no keys
    b = p2.find(c, lambda n: n.get_role_name() == "button" and n.get_name() == "Cancel")
    if b: p2.do_action(b)
time.sleep(1)
call = subprocess.Popen([sys.executable, os.path.join(p1.HERE, "gtk_call.py"), HOME], stdout=open(os.path.join(p1.OUT, "call.out"), "w"), stderr=subprocess.STDOUT)
for _ in range(100):
    d = p1.dialog()
    if d: break
    time.sleep(0.05)
time.sleep(1.0); d = p1.dialog(); ch = p2.choosers()[0]
say("start:", p2.read_folder(ch), "| keystrokes used in this probe: 0")
play = os.path.join(HOME, "Work/ofolder/spike/runs/h3/playground")
cases = [HOME + "/Documents", HOME + "/Work", play + "/ünïcödé 日本語", play + "/with space", play + "/quo'te\"d", play + "/-dash", play + "/.hidden",
         play + "/" + "/".join("d%d" % i for i in range(15)), HOME + "/Downloads", "/", "/usr/bin", HOME]
lats = []
for path in cases:
    if not os.path.isdir(path):
        say("SKIP missing", path[-40:]); continue
    ok, lat = goto(ch, path)
    say("%-6s action=%s latency=%s -> %s" % ("PASS" if lat is not None else "FAIL", ok, "%.0fms" % (lat * 1000) if lat is not None else "NONE", path[-48:]))
    if lat is not None: lats.append(lat)
    time.sleep(0.15)
if lats: say("INFO latency min=%.0fms max=%.0fms n=%d" % (min(lats) * 1000, max(lats) * 1000, len(lats)))
# informational: bad inputs must leave the reader honest (None or unchanged, never a wrong folder)
before = p2.read_folder(ch)
for label, bad in (("missing dir", "/nonexistent-xyz/"), ("a file, not a dir", "/etc/passwd")):
    e = entry(ch); Atspi.EditableText.set_text_contents(e, bad); Atspi.Action.do_action(e, 0); time.sleep(0.8)
    say("PROBE bad input %-18s reading before=%s after=%s windows=%d" % (label, before, p2.read_folder(ch), len(p2.choosers())))
    if len(p2.choosers()) == 0: break
for c in p2.choosers():
    b = p2.find(c, lambda n: n.get_role_name() == "button" and n.get_name() == "Cancel")
    if b: say("closing via Cancel ->", p2.do_action(b))
try: call.wait(5)
except subprocess.TimeoutExpired: call.kill()
say("call output:", open(os.path.join(p1.OUT, "call.out")).read().strip()[:160])
