#!/usr/bin/env python3
"""Probe: what can be observed from OUTSIDE the GTK portal's save dialog (xdg-desktop-portal-gtk)?

Calls the GTK backend's FileChooser.SaveFile directly (so portals.conf and your Flea default are untouched),
finds the dialog window, surveys the owning process before and after moving the dialog with wtype, and screenshots.
"""
import json, os, subprocess, sys, time

os.environ.setdefault("HYPRLAND_INSTANCE_SIGNATURE", "efb50993780079460b0cbed1363e2166a2de1d9f_1792084656_1809977280")
os.environ.setdefault("WAYLAND_DISPLAY", "wayland-1")
HERE = os.path.dirname(os.path.abspath(__file__))
OUT = sys.argv[1] if len(sys.argv) > 1 else os.path.join(HERE, "runs", "gtk1")
os.makedirs(OUT, exist_ok=True)
HOME = os.path.expanduser("~")
TITLE = "GTK spike save"


def say(*a):
    line = " ".join(str(x) for x in a)
    print(line, flush=True)
    open(os.path.join(OUT, "probe.txt"), "a").write(line + "\n")


def hj(*args):
    try:
        return json.loads(subprocess.run(["hyprctl", *args, "-j"], capture_output=True, text=True).stdout)
    except json.JSONDecodeError:
        return None


def dialog():
    return next((c for c in hj("clients") or [] if c["title"] == TITLE), None)


def dir_table():
    """inode -> [paths] for directories near HOME, to resolve inotify watches back to names."""
    table, n = {}, 0
    skip = {".cache", ".local", "node_modules", ".git", ".rustup", ".cargo", "go", ".npm"}
    def walk(p, depth):
        nonlocal n
        try:
            ents = list(os.scandir(p))
        except OSError:
            return
        for e in ents:
            if n > 40000:
                return
            try:
                if e.is_dir(follow_symlinks=False):
                    table.setdefault(e.stat(follow_symlinks=False).st_ino, []).append(e.path)
                    n += 1
                    if depth < 3 and e.name not in skip:
                        walk(e.path, depth + 1)
            except OSError:
                pass
    table.setdefault(os.stat("/").st_ino, []).append("/")
    table.setdefault(os.stat(HOME).st_ino, []).append(HOME)
    walk(HOME, 1)
    return table


def survey(tag, pid, table):
    say("=== survey", tag, "pid", pid)
    try:
        say("cwd:", os.readlink("/proc/%d/cwd" % pid))
        dirs = []
        for fd in os.listdir("/proc/%d/fd" % pid):
            t = os.readlink("/proc/%d/fd/%s" % (pid, fd))
            if t.startswith("/") and os.path.isdir(t):
                dirs.append(t)
        say("dir fds:", sorted(dirs))
        watched = []
        for fd in os.listdir("/proc/%d/fdinfo" % pid):
            for line in open("/proc/%d/fdinfo/%s" % (pid, fd)):
                if line.startswith("inotify"):
                    ino = int(line.split("ino:")[1].split()[0], 16)
                    watched.append(table.get(ino, ["?%x" % ino]))
        say("inotify watches resolved:", sorted({p for ps in watched for p in ps}))
    except OSError as e:
        say("survey error:", e)


def main():
    table = dir_table()
    say("dir table size", len(table))
    call = subprocess.Popen([sys.executable, os.path.join(HERE, "gtk_call.py"), HOME],
                            stdout=open(os.path.join(OUT, "call.out"), "w"), stderr=subprocess.STDOUT)
    d = None
    for _ in range(100):
        d = dialog()
        if d:
            break
        time.sleep(0.05)
    if not d:
        say("NO DIALOG WINDOW appeared; call output:", open(os.path.join(OUT, "call.out")).read()[:400])
        call.kill()
        return
    time.sleep(1.0)
    d = dialog()
    say("window: class=%r title=%r pid=%d at=%s size=%s floating=%s ws=%s" % (d["class"], d["title"], d["pid"], d["at"], d["size"], d["floating"], d["workspace"]["id"]))
    rect = "%d,%d %dx%d" % (d["at"][0], d["at"][1], d["size"][0], d["size"][1])
    subprocess.run(["grim", "-g", rect, os.path.join(OUT, "1-open.png")])
    survey("open at HOME", d["pid"], table)

    # Move the dialog: focus it, verify it is the active window, then type a folder path + Enter into the name entry.
    subprocess.run(["hyprctl", "dispatch", 'hl.dsp.focus({ window = "address:%s" })' % d["address"]], capture_output=True)
    time.sleep(0.4)
    active = (hj("activewindow") or {}).get("address")
    if active != d["address"]:
        say("REFUSED to type: dialog is not the active window")
    else:
        subprocess.run(["wtype", "--", HOME + "/Documents/"])
        time.sleep(0.3)
        subprocess.run(["wtype", "-k", "Return"])
        time.sleep(1.2)
        say("typed", HOME + "/Documents/", "+ Return")
        subprocess.run(["grim", "-g", rect, os.path.join(OUT, "2-after-documents.png")])
        survey("after Documents", d["pid"], table)
        subprocess.run(["wtype", "-k", "Escape"])
    time.sleep(1.0)
    try:
        call.wait(5)
    except subprocess.TimeoutExpired:
        say("dialog still open after Escape; killing the call")
        call.kill()
    say("call output:", open(os.path.join(OUT, "call.out")).read().strip()[:300])


if __name__ == "__main__":
    main()
