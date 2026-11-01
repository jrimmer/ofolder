"""End to end: client -> ofolder proxy (strip hooks on) -> real dialog; the strip appears docked, then everything cleans up."""
import json, os, subprocess, sys, tempfile, time
os.environ.setdefault("HYPRLAND_INSTANCE_SIGNATURE", "efb50993780079460b0cbed1363e2166a2de1d9f_1792084656_1809977280")
os.environ.setdefault("WAYLAND_DISPLAY", "wayland-1")
HERE = os.path.dirname(os.path.abspath(__file__))
sys.argv = [sys.argv[0], os.path.join(HERE, "runs", "proxy-strip-gtk")]
sys.path.insert(0, HERE)
import gtk_probe2 as p2
p1, say = p2.p1, p2.say
os.makedirs(p1.OUT, exist_ok=True)
scratch = tempfile.mkdtemp(prefix="ofolder-e2e-")
env = dict(os.environ, OFOLDER_FAVORITES=os.path.join(scratch, "favorites.json"))
BIN = os.path.join(HERE, "..", "target", "debug", "ofolder")
proxy = subprocess.Popen([BIN, "portal", "--backend", "gtk"], env=env, stdout=open(os.path.join(p1.OUT, "proxy.out"), "w"), stderr=open(os.path.join(p1.OUT, "proxy.err"), "w"))
time.sleep(1.5)
say("proxy:", open(os.path.join(p1.OUT, "proxy.out")).read().strip())
def clients(): return p1.hj("clients") or []
active_before = (p1.hj("activewindow") or {}).get("address")
call = subprocess.Popen([sys.executable, os.path.join(HERE, "portal_call.py"), "ofolder", p1.HOME, p1.TITLE], stdout=open(os.path.join(p1.OUT, "call.out"), "w"), stderr=subprocess.STDOUT)
dialog = strip = None
for _ in range(200):
    cs = clients()
    dialog = next((c for c in cs if c["title"] == p1.TITLE), None)
    strip = next((c for c in cs if c["class"] == "ofolder.strip"), None)
    if dialog and strip: break
    time.sleep(0.05)
time.sleep(1.5)
cs = clients()
dialog = next((c for c in cs if c["title"] == p1.TITLE), None); strip = next((c for c in cs if c["class"] == "ofolder.strip"), None)
say("dialog:", dialog and (dialog["at"], dialog["size"], dialog["floating"]))
say("strip:", strip and (strip["at"], strip["size"], strip["floating"], "pid", strip["pid"]))
if dialog and strip:
    want_x = dialog["at"][0] + dialog["size"][0] + 6
    say("DOCKED flush on the right:", strip["at"][0] == want_x and strip["at"][1] == dialog["at"][1] and strip["size"][1] == dialog["size"][1] and strip["floating"])
    x, y = min(dialog["at"][0], strip["at"][0]), min(dialog["at"][1], strip["at"][1])
    x2 = max(dialog["at"][0] + dialog["size"][0], strip["at"][0] + strip["size"][0]); y2 = max(dialog["at"][1] + dialog["size"][1], strip["at"][1] + strip["size"][1])
    subprocess.run(["grim", "-g", "%d,%d %dx%d" % (x, y, x2 - x, y2 - y), os.path.join(p1.OUT, "docked.png")])
active_after = (p1.hj("activewindow") or {}).get("address")
say("FOCUS stayed with the dialog (strip did not take it):", bool(dialog and active_after == dialog["address"]), "| strip has focus:", bool(strip and active_after == strip["address"]))
# move the dialog; the strip must follow
if dialog:
    subprocess.run(["hyprctl", "dispatch", 'hl.dsp.window.move({ x = 200, y = 100, relative = true, window = "address:%s" })' % dialog["address"]], capture_output=True)
    time.sleep(1.2)
    cs = clients()
    d2 = next((c for c in cs if c["title"] == p1.TITLE), None); s2 = next((c for c in cs if c["class"] == "ofolder.strip"), None)
    say("FOLLOWED the move:", bool(d2 and s2 and s2["at"][0] == d2["at"][0] + d2["size"][0] + 6 and s2["at"][1] == d2["at"][1]))
chs = p2.choosers()
say("cancel via accessibility ->", chs and p2.do_action(p2.find(chs[0], lambda n: n.get_role_name() == "button" and n.get_name() == "Cancel")))
try: call.wait(8)
except subprocess.TimeoutExpired: say("call did not finish"); call.kill()
say("client received:", open(os.path.join(p1.OUT, "call.out")).read().strip()[:80])
for _ in range(60):
    if not any(c["class"] == "ofolder.strip" for c in clients()): break
    time.sleep(0.1)
say("strip gone after the dialog closed:", not any(c["class"] == "ofolder.strip" for c in clients()))
say("strip processes left:", subprocess.run(["pgrep", "-fc", "^%s strip " % os.path.abspath(BIN)], capture_output=True, text=True).stdout.strip())
say("proxy alive:", proxy.poll() is None, "| stderr:", open(os.path.join(p1.OUT, "proxy.err")).read().strip()[:200] or "(none)")
proxy.terminate(); proxy.wait(5)
import shutil; shutil.rmtree(scratch, ignore_errors=True)
