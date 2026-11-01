#!/usr/bin/env python3
"""Probe 6: probe 5 alternated PASS/FAIL. Is it timing (pause) or consumed-first-activate (double)? Safe AT-SPI subset only."""
import os, subprocess, sys, time
import gi
gi.require_version("Atspi", "2.0")
from gi.repository import Atspi
sys.argv = [sys.argv[0], os.path.join(os.path.dirname(os.path.abspath(__file__)), "runs", "gtk6")]
import gtk_probe2 as p2
p1, say, HOME = p2.p1, p2.say, p2.HOME
os.makedirs(p1.OUT, exist_ok=True)
play = os.path.join(HOME, "Work/ofolder/spike/runs/h3/playground")
targets = [HOME + "/Documents", HOME + "/Work", play + "/ünïcödé 日本語", play + "/with space", play + "/quo'te\"d", play + "/-dash",
           play + "/.hidden", HOME + "/Downloads", "/", "/usr/bin", HOME + "/Music", HOME]
entry = lambda ch: next((t for t in p2.find_all(ch, lambda n: n.get_role_name() == "text") if t.get_state_set().contains(Atspi.StateType.EDITABLE)), None)
def run(ch, mode):
    ok = 0
    for path in targets:
        e = entry(ch)
        if mode == "pause": time.sleep(1.0)
        Atspi.EditableText.set_text_contents(e, path.rstrip("/") + "/")
        Atspi.Action.do_action(e, 0)
        if mode == "double":
            time.sleep(0.4); Atspi.Action.do_action(entry(ch), 0)
        lat = p2.wait_read(ch, path, 2.0)
        ok += lat is not None
        say("  %-7s %-6s %s" % (mode, "PASS" if lat is not None else "FAIL", path[-40:]))
    return ok
call = subprocess.Popen([sys.executable, os.path.join(p1.HERE, "gtk_call.py"), HOME], stdout=open(os.path.join(p1.OUT, "call.out"), "w"), stderr=subprocess.STDOUT)
for _ in range(100):
    if p1.dialog(): break
    time.sleep(0.05)
time.sleep(1.0); ch = p2.choosers()[0]
res = {"pass%d" % i: run(ch, "double") for i in (1, 2)}
say("RESULT passes of %d: %s" % (len(targets), res))
for c in p2.choosers():
    b = p2.find(c, lambda n: n.get_role_name() == "button" and n.get_name() == "Cancel")
    if b: say("closing via Cancel ->", p2.do_action(b))
try: call.wait(5)
except subprocess.TimeoutExpired: call.kill()
say("daemon alive:", subprocess.run(["pgrep", "-fc", "^/usr/lib/xdg-desktop-portal-gtk"], capture_output=True, text=True).stdout.strip())
