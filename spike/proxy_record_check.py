"""Live: confirm (Save) a real dialog through the proxy and check what the recents store holds. usage: proxy_record_check.py gtk|flea"""
import json, os, shutil, subprocess, sys, tempfile, time
BACKEND = sys.argv[1]
os.environ.setdefault("HYPRLAND_INSTANCE_SIGNATURE", "efb50993780079460b0cbed1363e2166a2de1d9f_1792084656_1809977280")
os.environ.setdefault("WAYLAND_DISPLAY", "wayland-1")
HERE = os.path.dirname(os.path.abspath(__file__))
sys.argv = [sys.argv[0], os.path.join(HERE, "runs", "proxy-record-" + BACKEND)]
sys.path.insert(0, HERE)
import gtk_probe2 as p2
p1, say = p2.p1, p2.say
shutil.rmtree(p1.OUT, ignore_errors=True); os.makedirs(p1.OUT, exist_ok=True)
scratch = tempfile.mkdtemp(prefix="ofolder-rec-")
saveto = os.path.join(scratch, "reports"); os.makedirs(saveto)
env = dict(os.environ, OFOLDER_RECENTS=os.path.join(scratch, "recents.json"), OFOLDER_SETTINGS=os.path.join(scratch, "settings.toml"),
           OFOLDER_FAVORITES=os.path.join(scratch, "favorites.json"), OFOLDER_DIALOGS=os.path.join(scratch, "dialogs.json"))
BIN = os.path.abspath(os.path.join(HERE, "..", "target", "debug", "ofolder"))
TITLE = "ofolder record " + BACKEND
CLASS = {"flea": "com.thisisgm.flea.picker", "gtk": "xdg-desktop-portal-gtk"}[BACKEND]
proxy = subprocess.Popen([BIN, "portal", "--backend", BACKEND], env=env, stdout=subprocess.DEVNULL, stderr=open(os.path.join(p1.OUT, "proxy.err"), "w"))
time.sleep(1.5)
def run_dialog(confirm):
    call = subprocess.Popen([sys.executable, os.path.join(HERE, "portal_call.py"), "ofolder", saveto, TITLE], stdout=open(os.path.join(p1.OUT, "call.out"), "w"), stderr=subprocess.STDOUT)
    d = None
    for _ in range(200):
        d = next((c for c in (p1.hj("clients") or []) if c["class"] == CLASS and (BACKEND == "flea" or c["title"] == TITLE)), None)
        if d: break
        time.sleep(0.05)
    time.sleep(2.0)
    if BACKEND == "gtk":
        desk = p2.Atspi.get_desktop(0)
        for i in range(desk.get_child_count()):
            a = desk.get_child_at_index(i)
            if a and (a.get_name() or "") == "xdg-desktop-portal-gtk":
                n = p2.find(a, lambda x: x.get_role_name() == "button" and x.get_name() == ("Save" if confirm else "Cancel"))
                say("pressed", "Save" if confirm else "Cancel", "->", n and p2.do_action(n))
    else:
        d = next((c for c in (p1.hj("clients") or []) if c["class"] == CLASS), None)
        subprocess.run(["hyprctl", "dispatch", 'hl.dsp.focus({ window = "address:%s" })' % d["address"]], capture_output=True); time.sleep(0.4)
        if (p1.hj("activewindow") or {}).get("address") == d["address"]:
            subprocess.run(["wtype", "-k", "Return" if confirm else "Escape"]); say("sent", "Return" if confirm else "Escape", "to the verified-active picker")
    try: call.wait(10)
    except subprocess.TimeoutExpired: call.kill(); say("call did not finish")
    out = open(os.path.join(p1.OUT, "call.out")).read().strip()
    say("client received:", out[:150])
    time.sleep(1.0)
    return out
rec = os.path.join(scratch, "recents.json")
out = run_dialog(False)
say("after CANCEL: recents file exists:", os.path.exists(rec))
out = run_dialog(True)
say("after SAVE: recents file exists:", os.path.exists(rec))
if os.path.exists(rec):
    data = json.load(open(rec))
    say("folders:", [(f["path"].replace(scratch, "<scratch>"), f["apps"]) for f in data["folders"]])
    say("files:", [(f["path"].replace(scratch, "<scratch>"), f["apps"]) for f in data["files"]])
    say("RECORDED the save folder:", any(f["path"] == saveto for f in data["folders"]), "| recorded the file:", any(f["path"].startswith(saveto + "/") for f in data["files"]))
say("proxy alive:", proxy.poll() is None, "| stderr:", (open(os.path.join(p1.OUT, "proxy.err")).read().strip()[:200] or "(none)"))
proxy.terminate(); proxy.wait(5)
say("files created inside the save folder (must be none):", os.listdir(saveto))
shutil.rmtree(scratch, ignore_errors=True)
