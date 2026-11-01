import os, subprocess, sys, time
sys.argv = [sys.argv[0], os.path.join(os.path.dirname(os.path.abspath(__file__)), "runs", "gtk4")]
import gtk_probe2 as p2
p1, say, HOME = p2.p1, p2.say, p2.HOME
os.makedirs(p1.OUT, exist_ok=True)
def shot(d, name):
    subprocess.run(["grim", "-g", "%d,%d %dx%d" % (d["at"][0], d["at"][1], d["size"][0], d["size"][1]), os.path.join(p1.OUT, name)])
def close_all():
    for ch in p2.choosers():
        c = p2.find(ch, lambda n: n.get_role_name() == "button" and n.get_name() == "Cancel")
        if c: say("cancel ->", p2.do_action(c))
if sys.argv and os.environ.get("ONLY_CLEAN"):
    close_all(); time.sleep(1); print("left:", len(p2.choosers())); raise SystemExit
close_all(); time.sleep(1)
call = subprocess.Popen([sys.executable, os.path.join(p1.HERE, "gtk_call.py"), HOME], stdout=open(os.path.join(p1.OUT, "call.out"), "w"), stderr=subprocess.STDOUT)
for _ in range(100):
    d = p1.dialog()
    if d: break
    time.sleep(0.05)
time.sleep(1.0); d = p1.dialog(); ch = p2.choosers()[0]
subprocess.run(["hyprctl", "dispatch", 'hl.dsp.focus({ window = "address:%s" })' % d["address"]], capture_output=True); time.sleep(0.4)
assert p2.active_is(d["address"]), "dialog not active"
subprocess.run(["wtype", "-M", "ctrl", "a", "-m", "ctrl"]); time.sleep(0.2)
subprocess.run(["wtype", "--", HOME + "/Documents/"]); time.sleep(0.8)
shot(d, "1-typed.png"); say("after typing: reading=", p2.read_folder(ch))
assert p2.active_is(d["address"]), "dialog lost focus"
subprocess.run(["wtype", "-k", "Return"]); time.sleep(1.0)
shot(d, "2-after-return.png"); say("after Return: reading=", p2.read_folder(ch), "| dialog windows:", len([c for c in p1.hj("clients") if c["title"] == p1.TITLE]))
close_all()
try: call.wait(5)
except subprocess.TimeoutExpired: call.kill()
say("left:", len(p2.choosers()))
