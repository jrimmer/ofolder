"""Notices when something ofolder leans on has changed, and says whether the suite should be run again.

It records the versions of Flea, Strata, the portal frontend and the GTK backend, the Hyprland that is
installed and the one that is running, and the state of the file-chooser chain, in
~/.local/state/ofolder/canary.json. `bin/canary` prints what changed since the last look and, with
`--run`, runs `bin/e2e --release` when something did. It changes nothing but that state file.
"""
import json
import os
import subprocess
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
STATE = os.path.join(os.environ.get("XDG_STATE_HOME") or os.path.expanduser("~/.local/state"), "ofolder", "canary.json")


def out(*cmd):
    try:
        r = subprocess.run(cmd, capture_output=True, text=True, timeout=15)
    except (OSError, subprocess.TimeoutExpired):
        return None
    return (r.stdout or r.stderr).strip().splitlines()[0] if r.returncode == 0 and (r.stdout or r.stderr).strip() else None


def pkg(name):
    return out("pacman", "-Q", name)


def running_hyprland():
    """The running session's version, found even when this shell has not exported its signature."""
    try:
        from lib import Host

        first = (Host().hypr("version").stdout or "").strip().splitlines()
    except SystemExit:
        return None
    return first[0].split(" built")[0] if first else None


def look():
    strata = os.path.expanduser("~/.local/bin/strata")
    return {
        "flea": out("flea", "--version"),
        "flea package": pkg("flea"),
        "strata": out(strata, "--version") if os.access(strata, os.X_OK) else None,
        "xdg-desktop-portal": pkg("xdg-desktop-portal"),
        "xdg-desktop-portal-gtk": pkg("xdg-desktop-portal-gtk"),
        "hyprland package": pkg("hyprland"),
        "hyprland installed": (out("Hyprland", "--version") or "").split(" built")[0],
        "hyprland running": running_hyprland(),
        "ofolder chain": out(os.path.expanduser("~/.local/bin/ofolder"), "status") and next(
            (l for l in subprocess.run([os.path.expanduser("~/.local/bin/ofolder"), "status"], capture_output=True, text=True,
                                       env=dict(os.environ, XDG_CURRENT_DESKTOP="Hyprland")).stdout.splitlines()
             if "chain" in l), None),
    }


def main():
    now = look()
    try:
        before = json.load(open(STATE))
    except (OSError, ValueError):
        before = None
    problems = []
    changed = []
    if before is None:
        print("First look: nothing to compare with yet.")
    else:
        changed = [(k, before.get(k), v) for k, v in now.items() if before.get(k) != v]
        for k, old, new in changed:
            print("changed  %-24s %s  ->  %s" % (k, old, new))
        if not changed:
            print("Nothing ofolder leans on has changed since the last look.")
    if now["hyprland installed"] != now["hyprland running"]:
        problems.append("Hyprland %s is installed but %s is running: the tests use the installed one, so log out and in to match what you will get."
                        % (now["hyprland installed"].replace("Hyprland ", ""), (now["hyprland running"] or "unknown").replace("Hyprland ", "")))
    chain = now.get("ofolder chain") or ""
    if "warning" in chain or "bypass" in chain or not chain:
        problems.append("ofolder is not leading the file-chooser chain: dialogs open without it. " + chain)
    for p in problems:
        print("WARNING:", p)
    os.makedirs(os.path.dirname(STATE), exist_ok=True)
    tmp = STATE + ".tmp"
    json.dump(now, open(tmp, "w"), indent=2)
    os.replace(tmp, STATE)
    if "--run" in sys.argv and (changed or before is None or "--force" in sys.argv):
        print("Running the suite on the release build...")
        sys.exit(subprocess.call([os.path.join(REPO, "bin", "e2e"), "--release"]))
    sys.exit(1 if problems else 0)


main()
