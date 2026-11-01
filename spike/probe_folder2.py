import json, os, subprocess, time
HOME = os.path.expanduser("~")
os.environ.update(HYPRLAND_INSTANCE_SIGNATURE="efb50993780079460b0cbed1363e2166a2de1d9f_1792084656_1809977280", WAYLAND_DISPLAY="wayland-1")

def descendants(pid):
    out, todo = [], [pid]
    while todo:
        p = todo.pop()
        kids = subprocess.run(["pgrep", "-P", str(p)], capture_output=True, text=True).stdout.split()
        out += [int(k) for k in kids]; todo += [int(k) for k in kids]
    return out

def current_folder(root_pid):
    """Directory fds held by the picker's process tree, excluding cwd."""
    found = []
    for p in [root_pid] + descendants(root_pid):
        try:
            for fd in os.listdir("/proc/%d/fd" % p):
                t = os.readlink("/proc/%d/fd/%s" % (p, fd))
                if t.startswith("/") and os.path.isdir(t) and t != os.readlink("/proc/%d/cwd" % p):
                    found.append((p, int(fd), t))
        except OSError:
            pass
    return found

req = {"mode": "save", "title": "Probe", "app": "probe", "accept": "", "multiple": False, "directory": False,
       "folder": HOME, "file": "", "name": "p.txt", "files": [], "filters": [], "current": "", "currentIndex": -1}
child = subprocess.Popen(["flea", "--pick", "/tmp/probe2-reply.json"], env=dict(os.environ, FLEA_PICKER=json.dumps(req)),
                         stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
time.sleep(3)
print("tree:", [(p, open("/proc/%d/cmdline" % p).read().replace("\0", " ")[:60]) for p in [child.pid] + descendants(child.pid)])
print("start        ->", current_folder(child.pid))
subprocess.run(["hyprctl", "dispatch", 'hl.dsp.focus({ window = "class:com.thisisgm.flea.picker" })'], capture_output=True)
time.sleep(0.3)
for target in (HOME + "/Work", HOME + "/Downloads", HOME + "/Documents"):
    subprocess.run(["wtype", "-M", "ctrl", "l", "-m", "ctrl"]); time.sleep(0.25)
    subprocess.run(["wtype", target]); time.sleep(0.15)
    subprocess.run(["wtype", "-k", "Return"]); t0 = time.time()
    seen = None
    while time.time() - t0 < 3:
        now = [t for _, _, t in current_folder(child.pid)]
        if target in now:
            seen = time.time() - t0; break
        time.sleep(0.01)
    print("-> %-26s latency=%s  readings=%s" % (target, "%.0fms" % (seen * 1000) if seen else "NOT SEEN", current_folder(child.pid)))
subprocess.run(["wtype", "-k", "Escape"]); time.sleep(1)
print("exit rc", child.poll())
