"""Live proof: client -> ofolder proxy -> real GTK portal dialog -> Cancel -> answer comes back through the proxy."""
import os, subprocess, sys, time
os.environ.setdefault("HYPRLAND_INSTANCE_SIGNATURE", "efb50993780079460b0cbed1363e2166a2de1d9f_1792084656_1809977280")
os.environ.setdefault("WAYLAND_DISPLAY", "wayland-1")
HERE = os.path.dirname(os.path.abspath(__file__))
sys.argv = [sys.argv[0], os.path.join(HERE, "runs", "proxy-gtk")]
sys.path.insert(0, HERE)
import gtk_probe2 as p2
p1, say = p2.p1, p2.say
os.makedirs(p1.OUT, exist_ok=True)
BIN = os.path.join(HERE, "..", "target", "debug", "ofolder")
proxy = subprocess.Popen([BIN, "portal", "--backend", "gtk"], stdout=open(os.path.join(p1.OUT, "proxy.out"), "w"), stderr=subprocess.STDOUT)
time.sleep(1.5)
say("proxy alive:", proxy.poll() is None, "|", open(os.path.join(p1.OUT, "proxy.out")).read().strip())
call = subprocess.Popen([sys.executable, os.path.join(HERE, "portal_call.py"), "ofolder", p1.HOME, p1.TITLE],
                        stdout=open(os.path.join(p1.OUT, "call.out"), "w"), stderr=subprocess.STDOUT)
d = None
for _ in range(120):
    d = p1.dialog()
    if d: break
    time.sleep(0.05)
say("a real GTK dialog opened through the proxy:", bool(d), "| class", d and d["class"], "| pid", d and d["pid"])
time.sleep(1.0)
chs = p2.choosers()
say("accessibility sees the dialog:", len(chs) == 1, "| reads folder:", chs and p2.read_folder(chs[0]))
cancel = chs and p2.find(chs[0], lambda n: n.get_role_name() == "button" and n.get_name() == "Cancel")
say("pressing Cancel via accessibility ->", cancel and p2.do_action(cancel))
try: call.wait(8)
except subprocess.TimeoutExpired: say("call did not finish; killing"); call.kill()
say("client received:", open(os.path.join(p1.OUT, "call.out")).read().strip()[:160])
say("proxy still alive after the dialog closed:", proxy.poll() is None)
proxy.terminate(); proxy.wait(5)
say("dialog windows left:", len([c for c in (p1.hj("clients") or []) if c["title"] == p1.TITLE]))
