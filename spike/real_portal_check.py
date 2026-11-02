"""Live, through the REAL portal frontend: a GTK4 test app (own window class, no portal id) asks for a Save dialog.

ofolder is registered temporarily as the first file-chooser backend, then everything is put back and verified by hash.
usage: real_portal_check.py            (driver)
       real_portal_check.py --app OUT   (the test app, started by the driver)
"""
import hashlib, json, os, shutil, subprocess, sys, tempfile, time

APP_ID = "org.ofolder.TestApp"
HERE = os.path.dirname(os.path.abspath(__file__))
BIN = os.path.abspath(os.path.join(HERE, "..", "target", "debug", "ofolder"))
HOME = os.path.expanduser("~")

if len(sys.argv) > 2 and sys.argv[1] == "--app":
    import gi
    gi.require_version("Gtk", "4.0")
    from gi.repository import Gtk, GLib
    out = sys.argv[2]
    app = Gtk.Application(application_id=APP_ID)

    def on_activate(a):
        win = Gtk.ApplicationWindow(application=a, title="ofolder test app")
        win.set_default_size(300, 120)
        win.present()

        def ask():
            dialog = Gtk.FileDialog()
            dialog.set_initial_name("real.txt")

            def done(d, res):
                try:
                    f = d.save_finish(res)
                    open(out, "w").write("SAVED " + f.get_path())
                except GLib.Error as e:
                    open(out, "w").write("CANCELLED " + str(e.message))
                a.quit()
            dialog.save(win, None, done)
            return False
        GLib.timeout_add(1500, ask)
    app.connect("activate", on_activate)
    app.run([])
    sys.exit(0)

os.environ.setdefault("HYPRLAND_INSTANCE_SIGNATURE", "efb50993780079460b0cbed1363e2166a2de1d9f_1792084656_1809977280")
os.environ.setdefault("WAYLAND_DISPLAY", "wayland-1")
sys.argv = [sys.argv[0], os.path.join(HERE, "runs", "real-portal")]
sys.path.insert(0, HERE)
import gtk_probe2 as p2
p1, say = p2.p1, p2.say
shutil.rmtree(p1.OUT, ignore_errors=True); os.makedirs(p1.OUT, exist_ok=True)

scratch = tempfile.mkdtemp(prefix="ofolder-real-")
target = os.path.join(scratch, "reports"); os.makedirs(target)
bk = os.path.join(scratch, "backup"); os.makedirs(bk)

CONF = os.path.join(HOME, ".config/xdg-desktop-portal/hyprland-portals.conf")
PORTAL_DIR = os.path.join(HOME, ".local/share/xdg-desktop-portal/portals")
SVC_DIR = os.path.join(HOME, ".local/share/dbus-1/services")
PORTAL_FILE = os.path.join(PORTAL_DIR, "ofolder.portal")
SVC_FILE = os.path.join(SVC_DIR, "org.freedesktop.impl.portal.desktop.ofolder.service")
OFOLDER_CFG = os.path.join(HOME, ".config/ofolder")
OFOLDER_DATA = os.path.join(HOME, ".local/share/ofolder")
SETTINGS = os.path.join(OFOLDER_CFG, "settings.toml")
RECENTS = os.path.join(OFOLDER_DATA, "recents.json")
pre_existing = {d: os.path.exists(d) for d in (OFOLDER_CFG, OFOLDER_DATA)}

def sha(p): return hashlib.sha256(open(p, "rb").read()).hexdigest()
conf_hash = sha(CONF)
shutil.copy2(CONF, os.path.join(bk, "hyprland-portals.conf"))
before_listing = (sorted(os.listdir(PORTAL_DIR)), sorted(os.listdir(SVC_DIR)))

def restart_portal():
    subprocess.run(["systemctl", "--user", "restart", "xdg-desktop-portal.service"], check=False)
    time.sleep(2.0)

def restore():
    subprocess.run([BIN, "resume"], capture_output=True)
    subprocess.run(["pkill", "-f", "%s portal" % BIN], capture_output=True)
    for f in (PORTAL_FILE, SVC_FILE):
        if os.path.exists(f): os.remove(f)
    if BACKEND == "strata":
        subprocess.run([os.path.expanduser("~/.local/bin/strata"), "--uninstall-portal"], capture_output=True, timeout=60)
    shutil.copy2(os.path.join(bk, "hyprland-portals.conf"), CONF)
    restart_portal()
    for d in (OFOLDER_CFG, OFOLDER_DATA):
        if not pre_existing[d]: shutil.rmtree(d, ignore_errors=True)
    if os.path.exists(SETTINGS) and pre_existing[OFOLDER_CFG]: pass

def clients(): return p1.hj("clients") or []
BACKEND = os.environ.get("RPC_BACKEND", "flea")
PICKER_CLASS = {"flea": "com.thisisgm.flea.picker", "gtk": "xdg-desktop-portal-gtk", "strata": "io.github.lgse.Strata.FileChooser"}[BACKEND]
def picker(): return next((c for c in clients() if c["class"] == PICKER_CLASS), None)
def panels(): return [c for c in clients() if c["class"].startswith("ofolder.strip.")]

def cursor():
    t = subprocess.run(["hyprctl", "cursorpos"], capture_output=True, text=True).stdout.strip().split(",")
    return int(t[0]), int(t[1])

def click(x, y, address):
    hl = lambda lua: subprocess.run(["hyprctl", "dispatch", lua], capture_output=True, text=True).stdout.strip()
    # A real mouse sends motion before a press; a single warp may not reach the client as hover.
    for dx, dy in ((-40, -25), (-12, -8), (-3, -2), (0, 0)):
        hl("hl.dsp.cursor.move({ x = %d, y = %d })" % (x + dx, y + dy)); time.sleep(0.08)
    time.sleep(0.25)
    dbg = (cursor(), (p1.hj("activewindow") or {}).get("class"))
    r = hl('hl.dsp.send_shortcut({ mods = "", key = "mouse:272", window = "address:%s" })' % address)
    time.sleep(0.3)
    say("   click at", (x, y), "cursor", dbg[0], "active before", dbg[1], "| active after", (p1.hj("activewindow") or {}).get("class"),
        "| window under panels:", [(c["class"], c["at"], c["size"]) for c in panels()])
    return r

def clip():
    r = subprocess.run(["wl-paste", "-n"], capture_output=True, text=True, timeout=5)
    return r.stdout if r.returncode == 0 else None

def focus_panel(panel):
    subprocess.run(["hyprctl", "dispatch", 'hl.dsp.focus({ window = "address:%s" })' % panel["address"]], capture_output=True); time.sleep(0.4)

def active_class():
    return (p1.hj("activewindow") or {}).get("class")

def type_into(panel_class, text, key=None):
    # Never type into a window that is not verified to be the target.
    for _ in range(20):
        if active_class() == panel_class: break
        time.sleep(0.1)
    if active_class() != panel_class:
        say("REFUSED to type: active window is", active_class(), "not", panel_class); return False
    if text: subprocess.run(["wtype", text])
    if key: subprocess.run(["wtype", "-k", key])
    return True

SCALE = (p1.hj("monitors") or [{"scale": 1.0}])[0]["scale"]

def off(panel, x_px, y_px):
    # The offsets were read from screenshots (physical pixels); Hyprland places windows in logical units.
    return panel["at"][0] + int(x_px / SCALE), panel["at"][1] + int(y_px / SCALE)

def panel_of(ps, side): return next(c for c in panels() if c["class"].endswith("." + side))

def flea_now():
    if BACKEND != "flea": return []
    import glob
    found = []
    for pid in subprocess.run(["pgrep", "-f", "flea --backend"], capture_output=True, text=True).stdout.split():
        for fd in glob.glob("/proc/%s/fd/*" % pid):
            try:
                t = os.readlink(fd)
                if t.startswith("/") and os.path.isdir(t) and (t.startswith(HOME) or t.startswith("/tmp/ofolder")) and ".config" not in t and ".local" not in t and ".cache" not in t: found.append(t.replace(scratch, "<scratch>"))
            except OSError: pass
    return sorted(set(found))

def nav(d, ps):
    L = panel_of(ps, "left")
    for name, x, y in (("Downloads", 108, 248), ("Parent", 188, 25), ("Back", 37, 25)):
        say("nav | click", name, "->", click(*off(L, x, y), L["address"])); 
        seen = []
        for _ in range(8):
            time.sleep(0.5); seen.append(flea_now())
        say("nav |   after", name, "the dialog's folder over 4s:", seen)
        subprocess.run(["grim", "-g", "%d,%d %dx%d" % (L["at"][0], L["at"][1], 1500, 120), os.path.join(p1.OUT, "nav-%s.png" % name)])
        L = panel_of(ps, "left")

def find(d, ps):
    R = panel_of(ps, "right")
    say("find | click box ->", click(*off(R, 100, 207), R["address"])); time.sleep(0.8)
    focus_panel(R)
    if type_into("ofolder.strip.right", "Downl"): time.sleep(1.2)
    subprocess.run(["grim", "-g", "%d,%d %dx%d" % (R["at"][0], R["at"][1], R["size"][0], R["size"][1]), os.path.join(p1.OUT, "find-typed.png")])
    type_into("ofolder.strip.right", "", "Return"); time.sleep(2.8)

def newfolder(d, ps):
    R = panel_of(ps, "right")
    say("newfolder | click ->", click(*off(R, 57, 96), R["address"])); time.sleep(0.8)
    focus_panel(R)
    if type_into("ofolder.strip.right", "made-by-test"):
        subprocess.run(["grim", "-g", "%d,%d %dx%d" % (R["at"][0], R["at"][1], R["size"][0], R["size"][1]), os.path.join(p1.OUT, "newfolder-typed.png")])
        type_into("ofolder.strip.right", "", "Return"); time.sleep(3.0)
    made = os.path.join(target, "made-by-test")
    say("newfolder | folder exists in the scratch folder:", os.path.isdir(made), "| nothing else created:", sorted(os.listdir(target)))

def terminal(d, ps):
    R = panel_of(ps, "right")
    before = {c["address"] for c in clients()}
    say("terminal | click ->", click(*off(R, 50, 133), R["address"])); time.sleep(3.0)
    new = [c for c in clients() if c["address"] not in before and not c["class"].startswith("ofolder.")]
    for c in new:
        try: cwd = os.readlink("/proc/%d/cwd" % c["pid"])
        except OSError: cwd = "?"
        kids = subprocess.run(["pgrep", "-P", str(c["pid"])], capture_output=True, text=True).stdout.split()
        kcwd = [os.readlink("/proc/%s/cwd" % k) for k in kids if os.path.exists("/proc/%s/cwd" % k)]
        say("terminal | new window:", c["class"], "| cwd:", cwd.replace(scratch, "<scratch>"), "| children cwd:", [k.replace(scratch, "<scratch>") for k in kcwd],
            "| started in the dialog's folder:", target in [cwd] + kcwd)
        subprocess.run(["hyprctl", "dispatch", 'hl.dsp.window.close({ window = "address:%s" })' % c["address"]], capture_output=True)
    if not new: say("terminal | NO new window appeared")

def clicks(d, ps):
    R = next(c for c in ps if c["class"].endswith(".right"))
    saved = clip(); home = cursor()
    say("click | Favorite ->", click(*off(R, 146, 59), R["address"])); time.sleep(1.2)
    fav = os.path.join(OFOLDER_CFG, "favorites.json")
    say("click | favorites file:", open(fav).read().replace(scratch, "<scratch>")[:200] if os.path.exists(fav) else "(none)")
    say("click | Copy path ->", click(*off(R, 55, 59), R["address"])); time.sleep(1.0)
    say("click | clipboard now:", (clip() or "(none)").replace(scratch, "<scratch>"))
    subprocess.run(["grim", "-g", "%d,%d %dx%d" % (R["at"][0], R["at"][1], R["size"][0], R["size"][1]), os.path.join(p1.OUT, "after-clicks.png")])
    if saved is not None: subprocess.run(["wl-copy"], input=saved, text=True)
    subprocess.run(["hyprctl", "dispatch", "hl.dsp.cursor.move({ x = %d, y = %d })" % home], capture_output=True)

def run_app(label, key, during=None):
    out = os.path.join(scratch, "app-%s.txt" % label)
    app = subprocess.Popen([sys.executable, os.path.abspath(__file__), "--app", out],
                           env=dict(os.environ, GTK_USE_PORTAL="1"), stdout=open(os.path.join(p1.OUT, "app-%s.log" % label), "w"), stderr=subprocess.STDOUT)
    # The test app must hold focus when it asks (Hyprland follows the mouse), so park the cursor on it.
    for _ in range(100):
        w = next((c for c in clients() if c["class"] == APP_ID), None)
        if w: break
        time.sleep(0.05)
    if w:
        subprocess.run(["hyprctl", "dispatch", "hl.dsp.cursor.move({ x = %d, y = %d })" % (w["at"][0] + w["size"][0] // 2, w["at"][1] + w["size"][1] // 2)], capture_output=True)
        subprocess.run(["hyprctl", "dispatch", 'hl.dsp.focus({ window = "address:%s" })' % w["address"]], capture_output=True)
    for _ in range(300):
        if picker() and len(panels()) == 2: break
        time.sleep(0.05)
    time.sleep(2.0)
    d = picker(); ps = panels()
    say(label, "| picker:", bool(d), "| panels:", len(ps), "| strips after request:", len(ps))
    if d:
        act = (p1.hj("activewindow") or {}).get("address")
        say(label, "| focus stayed with the dialog:", act == d["address"])
        subprocess.run(["grim", "-g", "%d,%d %dx%d" % (min([d["at"][0]] + [c["at"][0] for c in ps]), d["at"][1],
                        max(c["at"][0] + c["size"][0] for c in ps + [d]) - min([d["at"][0]] + [c["at"][0] for c in ps]), d["size"][1]),
                        os.path.join(p1.OUT, "%s.png" % label)])
        if during: during(d, ps)
        subprocess.run(["hyprctl", "dispatch", 'hl.dsp.focus({ window = "address:%s" })' % d["address"]], capture_output=True); time.sleep(0.4)
        if BACKEND == "gtk":
            name = "Save" if key == "Return" else "Cancel"
            desk = p2.Atspi.get_desktop(0); pressed = None
            for i in range(desk.get_child_count()):
                a = desk.get_child_at_index(i)
                if a and (a.get_name() or "") == "xdg-desktop-portal-gtk":
                    n = p2.find(a, lambda x: x.get_role_name() == "button" and x.get_name() == name)
                    if n: pressed = p2.do_action(n)
            say(label, "| pressed", name, "->", pressed)
        elif (p1.hj("activewindow") or {}).get("address") == d["address"]:
            subprocess.run(["wtype", "-k", key]); say(label, "| sent", key, "to the verified-active picker")
        else:
            say(label, "| REFUSED to type: the picker is not the active window")
    try: app.wait(15)
    except subprocess.TimeoutExpired:
        app.kill(); say(label, "| app did not finish")
    time.sleep(1.0)
    res = open(out).read() if os.path.exists(out) else "(no result)"
    say(label, "| app received:", res.replace(scratch, "<scratch>"))
    say(label, "| panels gone:", not panels())
    subprocess.run(["hyprctl", "dispatch", "hl.dsp.cursor.move({ x = %d, y = %d })" % START_CURSOR], capture_output=True)
    return res

START_CURSOR = cursor()
ok = False
try:
    if BACKEND == "strata":
        print(subprocess.run([os.path.expanduser("~/.local/bin/strata"), "--install-portal"], capture_output=True, text=True, timeout=60).stdout.strip()[:80])
        # Strata's own installer rewrote the config; ofolder goes first in the file it left, keeping its entries after.
        bk_conf = open(CONF).read()
    os.makedirs(OFOLDER_CFG, exist_ok=True)
    open(SETTINGS, "w").write('[start_folders]\n"class:%s" = "%s"\n' % (APP_ID, target))
    shutil.copy2(os.path.join(HERE, "..", "packaging", "ofolder.portal"), PORTAL_FILE)
    svc = open(os.path.join(HERE, "..", "packaging", "org.freedesktop.impl.portal.desktop.ofolder.service")).read().replace("@OFOLDER@", BIN)
    open(SVC_FILE, "w").write(svc)
    conf = open(CONF).read()
    assert "FileChooser=flea;gtk" in conf, conf
    open(CONF, "w").write(conf.replace("FileChooser=flea;gtk", "FileChooser=ofolder;%s" % {"gtk": "gtk;flea", "flea": "flea;gtk", "strata": "strata;flea;gtk"}[BACKEND]))
    restart_portal()
    say("registered: ofolder is first for FileChooser; portal frontend restarted")

    r1 = run_app("save", "Return")
    say("START FOLDER seeded from the class rule:", r1.startswith("SAVED") and r1.split(" ", 1)[1] == os.path.join(target, "real.txt"))
    if os.path.exists(RECENTS):
        data = json.load(open(RECENTS))
        say("recents folders:", [(f["path"].replace(scratch, "<scratch>"), f["apps"]) for f in data["folders"]])
        say("recents files:", [(f["path"].replace(scratch, "<scratch>"), f["apps"]) for f in data["files"]])
    else:
        say("NO recents file written")
    n_before = len(json.load(open(RECENTS))["folders"]) if os.path.exists(RECENTS) else 0

    r2 = run_app("cancel", "Escape")
    n_after = len(json.load(open(RECENTS))["folders"]) if os.path.exists(RECENTS) else 0
    say("cancel recorded nothing new:", n_before == n_after, "| files created in the save folder:", os.listdir(target))

    if "--clicks" in os.environ.get("RPC_MODE", ""):
        only = os.environ.get("RPC_ONLY")
        rn = run_app("nav", "Return", during=nav)
        if only == "nav": raise SystemExit
        say("NAV: Downloads -> Parent -> Back confirms in ~/Downloads:", rn == "SAVED " + os.path.join(HOME, "Downloads", "real.txt"))
        rf = run_app("find", "Return", during=find)
        say("FIND: typing Downl + Enter confirms in ~/Downloads:", rf == "SAVED " + os.path.join(HOME, "Downloads", "real.txt"))
        run_app("newfolder", "Escape", during=newfolder)
        run_app("terminal", "Escape", during=terminal)
        run_app("clicks", "Escape", during=clicks)
    subprocess.run([BIN, "pause"], capture_output=True)
    r3 = run_app("paused", "Escape")
    ok = True
finally:
    restore()
    say("== RESTORED ==")
    say("portal conf hash unchanged:", sha(CONF) == conf_hash)
    say("service/portal dirs unchanged:", (sorted(os.listdir(PORTAL_DIR)), sorted(os.listdir(SVC_DIR))) == before_listing)
    say("ofolder data dirs removed again:", all(os.path.exists(d) == pre_existing[d] for d in (OFOLDER_CFG, OFOLDER_DATA)))
    shutil.rmtree(scratch, ignore_errors=True)
say("completed:", ok)
