import json, os, subprocess, sys, time
os.environ.setdefault("HYPRLAND_INSTANCE_SIGNATURE", "efb50993780079460b0cbed1363e2166a2de1d9f_1792084656_1809977280")
os.environ.setdefault("WAYLAND_DISPLAY", "wayland-1")
HERE = os.path.dirname(os.path.abspath(__file__)); OUT = os.path.join(HERE, "runs", "strata1"); os.makedirs(OUT, exist_ok=True)
TITLE = "Strata spike save"; HOME = os.path.expanduser("~")
def say(*a):
    l = " ".join(str(x) for x in a); print(l, flush=True); open(os.path.join(OUT, "probe.txt"), "a").write(l + "\n")
def hj(*a):
    try: return json.loads(subprocess.run(["hyprctl", *a, "-j"], capture_output=True, text=True).stdout)
    except json.JSONDecodeError: return None
before = {c["address"] for c in hj("clients") or []}
call = subprocess.Popen([sys.executable, os.path.join(HERE, "portal_call.py"), "strata", HOME, TITLE], stdout=open(os.path.join(OUT, "call.out"), "w"), stderr=subprocess.STDOUT)
new = None
for _ in range(160):
    new = [c for c in hj("clients") or [] if c["address"] not in before]
    if new: break
    time.sleep(0.05)
time.sleep(1.5)
new = [c for c in hj("clients") or [] if c["address"] not in before]
if not new:
    say("NO WINDOW. call output:", open(os.path.join(OUT, "call.out")).read()[:500]); call.kill(); sys.exit()
for c in new:
    say("window: class=%r title=%r pid=%d at=%s size=%s floating=%s" % (c["class"], c["title"], c["pid"], c["at"], c["size"], c["floating"]))
d = new[0]; pid = d["pid"]
say("process:", open("/proc/%d/cmdline" % pid).read().replace("\0", " ")[:100], "| ppid", open("/proc/%d/stat" % pid).read().split()[3])
kids = subprocess.run(["pgrep", "-P", str(pid)], capture_output=True, text=True).stdout.split()
say("children:", [(k, open("/proc/%s/cmdline" % k).read().replace("\0", " ")[:60]) for k in kids])
dirs = []
for p in [pid] + [int(k) for k in kids]:
    try:
        cwd = os.readlink("/proc/%d/cwd" % p)
        for fd in os.listdir("/proc/%d/fd" % p):
            t = os.readlink("/proc/%d/fd/%s" % (p, fd))
            if t.startswith("/") and os.path.isdir(t) and t != cwd: dirs.append((p, fd, t))
    except OSError: pass
say("dir fds:", dirs)
subprocess.run(["grim", "-g", "%d,%d %dx%d" % (d["at"][0], d["at"][1], d["size"][0], d["size"][1]), os.path.join(OUT, "1-open.png")])
# accessibility: is it on the bus, and what does it expose?
import gi
try:
    gi.require_version("Atspi", "2.0"); from gi.repository import Atspi; Atspi.init()
    desk = Atspi.get_desktop(0)
    names = [desk.get_child_at_index(i).get_name() for i in range(desk.get_child_count())]
    say("a11y apps:", names)
    app = next((desk.get_child_at_index(i) for i in range(desk.get_child_count()) if "trata" in (desk.get_child_at_index(i).get_name() or "")), None)
    def walk(n, depth=0, out=[0]):
        if out[0] > 80 or depth > 12: return
        out[0] += 1
        st = n.get_state_set()
        fl = "".join(c for c, s in (("C", Atspi.StateType.CHECKED), ("S", Atspi.StateType.SELECTED), ("V", Atspi.StateType.VISIBLE)) if st.contains(s))
        say("  " * depth + "%s %r [%s]" % (n.get_role_name(), n.get_name(), fl))
        for j in range(min(n.get_child_count(), 12)): walk(n.get_child_at_index(j), depth + 1, out)
    if app: walk(app)
    else: say("Strata not on the a11y bus")
except Exception as e:
    say("atspi error:", e)
say("=== leaving the dialog open for the next probe (call pid %d)" % call.pid)
open(os.path.join(OUT, "callpid"), "w").write(str(call.pid))
