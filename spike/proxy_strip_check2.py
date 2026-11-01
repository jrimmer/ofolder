"""End to end for any backend: client -> ofolder proxy (strip hooks on) -> the real dialog. usage: proxy_strip_check2.py flea|gtk|strata"""
import os, subprocess, sys, tempfile, time, shutil
BACKEND = sys.argv[1]
os.environ.setdefault("HYPRLAND_INSTANCE_SIGNATURE", "efb50993780079460b0cbed1363e2166a2de1d9f_1792084656_1809977280")
os.environ.setdefault("WAYLAND_DISPLAY", "wayland-1")
HERE = os.path.dirname(os.path.abspath(__file__))
sys.argv = [sys.argv[0], os.path.join(HERE, "runs", "proxy-strip-" + BACKEND)]
sys.path.insert(0, HERE)
import gtk_probe2 as p2
import gi
gi.require_version("Atspi", "2.0")
from gi.repository import Atspi
p1, say = p2.p1, p2.say
shutil.rmtree(p1.OUT, ignore_errors=True); os.makedirs(p1.OUT, exist_ok=True)
TITLE = "ofolder e2e " + BACKEND
CLASS = {"flea": "com.thisisgm.flea.picker", "gtk": "xdg-desktop-portal-gtk", "strata": "io.github.lgse.Strata.FileChooser"}[BACKEND]
scratch = tempfile.mkdtemp(prefix="ofolder-e2e-")
env = dict(os.environ, OFOLDER_FAVORITES=os.path.join(scratch, "favorites.json"))
BIN = os.path.abspath(os.path.join(HERE, "..", "target", "debug", "ofolder"))
proxy = subprocess.Popen([BIN, "portal", "--backend", BACKEND], env=env, stdout=open(os.path.join(p1.OUT, "proxy.out"), "w"), stderr=open(os.path.join(p1.OUT, "proxy.err"), "w"))
time.sleep(1.5)
def clients(): return p1.hj("clients") or []
def dlg(cs): return next((c for c in cs if c["class"] == CLASS and (BACKEND == "flea" or c["title"] == TITLE)), None)
def panel(cs, side): return next((c for c in cs if c["class"] == "ofolder.strip." + side), None)
def both(cs): return panel(cs, "left") and panel(cs, "right")
call = subprocess.Popen([sys.executable, os.path.join(HERE, "portal_call.py"), "ofolder", scratch, TITLE], stdout=open(os.path.join(p1.OUT, "call.out"), "w"), stderr=subprocess.STDOUT)
for _ in range(240):
    cs = clients()
    if dlg(cs) and both(cs): break
    time.sleep(0.05)
time.sleep(2.0)
cs = clients(); d, L, R = dlg(cs), panel(cs, "left"), panel(cs, "right")
say(BACKEND, "dialog:", d and (d["at"], d["size"], "floating" if d["floating"] else "TILED"))
say(BACKEND, "left panel:", L and (L["at"], L["size"], "floating" if L["floating"] else "TILED"))
say(BACKEND, "right panel:", R and (R["at"], R["size"], "floating" if R["floating"] else "TILED"))
def flush(d, L, R):
    return bool(d and L and R and L["floating"] and R["floating"]
                and L["at"][0] + L["size"][0] + 6 == d["at"][0] and R["at"][0] == d["at"][0] + d["size"][0] + 6
                and L["at"][1] == d["at"][1] == R["at"][1] and L["size"][1] == d["size"][1] == R["size"][1])
if d and L and R:
    say("DOCKED: left panel flush on the left, right panel flush on the right:", flush(d, L, R))
    act = (p1.hj("activewindow") or {}).get("address")
    say("FOCUS stayed with the dialog:", act == d["address"], "| a panel has focus:", act in (L["address"], R["address"]))
    xs = [c["at"][0] for c in (d, L, R)]; ys = [c["at"][1] for c in (d, L, R)]
    x2 = max(c["at"][0] + c["size"][0] for c in (d, L, R)); y2 = max(c["at"][1] + c["size"][1] for c in (d, L, R))
    subprocess.run(["grim", "-g", "%d,%d %dx%d" % (min(xs), min(ys), x2 - min(xs), y2 - min(ys)), os.path.join(p1.OUT, "docked.png")])
    subprocess.run(["hyprctl", "dispatch", 'hl.dsp.window.move({ x = 200, y = 100, relative = true, window = "address:%s" })' % d["address"]], capture_output=True)
    time.sleep(1.5)
    cs = clients()
    say("FOLLOWED the move (both panels):", flush(dlg(cs), panel(cs, "left"), panel(cs, "right")))
# close the dialog the safe way for each backend
def a11y_cancel(app_hint, name):
    desk = Atspi.get_desktop(0)
    for i in range(desk.get_child_count()):
        a = desk.get_child_at_index(i)
        if a and app_hint in (a.get_name() or ""):
            n = p2.find(a, lambda x: x.get_role_name() == "button" and x.get_name() == name)
            if n: return p2.do_action(n)
if BACKEND == "gtk":
    chs = [c for c in p2.choosers_for(TITLE)] if hasattr(p2, "choosers_for") else []
    say("cancel via accessibility ->", a11y_cancel("xdg-desktop-portal-gtk", "Cancel"))
elif BACKEND == "strata":
    say("cancel via accessibility ->", a11y_cancel("Strata.FileChooser", "Cancel file selection (Escape)"))
else:
    d = dlg(clients())
    if d:
        subprocess.run(["hyprctl", "dispatch", 'hl.dsp.focus({ window = "address:%s" })' % d["address"]], capture_output=True); time.sleep(0.4)
        if (p1.hj("activewindow") or {}).get("address") == d["address"]:
            subprocess.run(["wtype", "-k", "Escape"]); say("sent Escape to the verified-active picker")
        else:
            say("REFUSED to type: the picker is not active")
try: call.wait(10)
except subprocess.TimeoutExpired: say("call did not finish; killing"); call.kill()
say("client received:", open(os.path.join(p1.OUT, "call.out")).read().strip()[:90])
for _ in range(80):
    if not (panel(clients(), "left") or panel(clients(), "right")): break
    time.sleep(0.1)
say("panels gone after the dialog closed:", not (panel(clients(), "left") or panel(clients(), "right")))
say("strip processes left:", subprocess.run(["pgrep", "-fc", "^%s strip " % BIN], capture_output=True, text=True).stdout.strip())
say("proxy alive:", proxy.poll() is None, "| stderr:", (open(os.path.join(p1.OUT, "proxy.err")).read().strip()[:160] or "(none)"))
proxy.terminate(); proxy.wait(5)
shutil.rmtree(scratch, ignore_errors=True)
