import json, os, subprocess, sys, time
os.environ.setdefault("HYPRLAND_INSTANCE_SIGNATURE", "efb50993780079460b0cbed1363e2166a2de1d9f_1792084656_1809977280")
os.environ.setdefault("WAYLAND_DISPLAY", "wayland-1")
import gi
gi.require_version("Atspi", "2.0")
from gi.repository import Atspi
Atspi.init()
HERE = os.path.dirname(os.path.abspath(__file__)); OUT = os.path.join(HERE, "runs", "strata2"); os.makedirs(OUT, exist_ok=True)
HOME = os.path.expanduser("~")
def say(*a):
    l = " ".join(str(x) for x in a); print(l, flush=True); open(os.path.join(OUT, "probe.txt"), "a").write(l + "\n")
def hj(*a):
    try: return json.loads(subprocess.run(["hyprctl", *a, "-j"], capture_output=True, text=True).stdout)
    except json.JSONDecodeError: return None
def app():
    d = Atspi.get_desktop(0)
    return next((d.get_child_at_index(i) for i in range(d.get_child_count()) if "Strata.FileChooser" in (d.get_child_at_index(i).get_name() or "")), None)
def dfs(n, out=None):
    out = [] if out is None else out
    out.append(n)
    for j in range(n.get_child_count()): dfs(n.get_child_at_index(j), out)
    return out
def read_folder():
    """Join the breadcrumb labels that share a container with the 'Copy path' button: ~ / Work / ofolder -> ~/Work/ofolder."""
    try:
        cp = next(n for n in dfs(app()) if n.get_role_name() == "button" and n.get_name() == "Copy path")
        anc = cp.get_parent()
        for _ in range(4):
            t = "".join(n.get_name() for n in dfs(anc) if n.get_role_name() == "label")
            if t.startswith("~") or t.startswith("/"):
                p = HOME + t[1:] if t.startswith("~") else t
                return p if os.path.isdir(p) else None
            anc = anc.get_parent()
    except Exception:
        return None

def click(name, role="button"):
    n = next((x for x in dfs(app()) if x.get_role_name() == role and x.get_name() == name), None)
    return Atspi.Action.do_action(n, 0) if n else None
def wait(want, timeout=2.5):
    t0 = time.time()
    while time.time() - t0 < timeout:
        if read_folder() == want: return time.time() - t0
        time.sleep(0.01)
def dialog():
    return next((c for c in hj("clients") or [] if c["class"] == "io.github.lgse.Strata.FileChooser"), None)
def active_is(a): return (hj("activewindow") or {}).get("address") == a
d = dialog()
if not d: say("no dialog open"); sys.exit()
A = '"address:%s"' % d["address"]
say("start: floating=%s size=%s reading=%s" % (d["floating"], d["size"], read_folder()))
# adapter job #1: make the dialog floating and a sane size (it opened tiled and squashed)
subprocess.run(["hyprctl", "dispatch", 'hl.dsp.window.float({ action = "on", window = %s })' % A], capture_output=True)
subprocess.run(["hyprctl", "dispatch", "hl.dsp.window.resize({ x = 1000, y = 640, relative = false, window = %s })" % A], capture_output=True)
subprocess.run(["hyprctl", "dispatch", "hl.dsp.window.move({ x = 1500, y = 300, relative = false, window = %s })" % A], capture_output=True)
time.sleep(1.0); d = dialog(); say("after float: floating=%s at=%s size=%s" % (d["floating"], d["at"], d["size"]))
subprocess.run(["grim", "-g", "%d,%d %dx%d" % (d["at"][0], d["at"][1], d["size"][0], d["size"][1]), os.path.join(OUT, "1-floated.png")])
def lat(want): 
    t = wait(want); return "PASS %.0fms" % (t * 1000) if t is not None else "FAIL reading=%s" % read_folder()
say("click place Documents ->", click("Documents"), lat(HOME + "/Documents"))
say("click place Downloads ->", click("Downloads"), lat(HOME + "/Downloads"))
say("click Parent folder ->", click("Parent folder (Alt+↑)"), lat(HOME))
say("click place Home ->", click("Home"), lat(HOME))
# Ctrl+L then type a path, only into the verified-active dialog
def goto_keys(path):
    subprocess.run(["hyprctl", "dispatch", 'hl.dsp.focus({ window = %s })' % A], capture_output=True); time.sleep(0.3)
    if not active_is(d["address"]): return "REFUSED (not active)"
    subprocess.run(["wtype", "-M", "ctrl", "l", "-m", "ctrl"]); time.sleep(0.3)
    if not active_is(d["address"]): return "REFUSED (lost focus)"
    subprocess.run(["wtype", "--", path]); time.sleep(0.2)
    if not active_is(d["address"]): return "REFUSED (lost focus)"
    subprocess.run(["wtype", "-k", "Return"]); return "sent"
play = os.path.join(HOME, "Work/ofolder/spike/runs/h3/playground")
for path in [HOME + "/Work", play + "/ünïcödé 日本語", play + "/with space", play + "/-dash", play + "/.hidden", "/usr/bin", HOME]:
    s = goto_keys(path)
    say("ctrl+L %-40s %s %s" % (path[-38:], s, lat(path) if s == "sent" else ""))
subprocess.run(["grim", "-g", "%d,%d %dx%d" % (d["at"][0], d["at"][1], d["size"][0], d["size"][1]), os.path.join(OUT, "2-after.png")])
say("click New Folder ->", click("New Folder"))
time.sleep(0.8)
subprocess.run(["grim", "-g", "%d,%d %dx%d" % (d["at"][0], d["at"][1], d["size"][0], d["size"][1]), os.path.join(OUT, "3-new-folder-ui.png")])
ents = [(n.get_role_name(), n.get_name()) for n in dfs(app()) if n.get_role_name() in ("text", "entry", "password text")]
say("editable-ish nodes now:", ents)
say("closing: click Cancel ->", click("Cancel file selection (Escape)"))
time.sleep(1.2)
cp = open(os.path.join(OUT, "..", "strata1", "callpid")).read().strip() if os.path.exists(os.path.join(OUT, "..", "strata1", "callpid")) else None
say("dialog windows left:", 1 if dialog() else 0, "| call process alive:", cp and os.path.exists("/proc/%s" % cp))
