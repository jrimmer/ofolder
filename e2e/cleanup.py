"""Removes what a killed e2e run can leave: the virtual output, its nested compositor and its folders.

Only things this suite names are touched: the output ofe2e-1, and /tmp/ofe2e.* runtime directories
whose processes it ends (a process is ours when its environment points at that directory).
"""
import glob
import os
import shutil
import signal
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from lib import OUTPUT, Host, say


def holders(rt):
    found = []
    for p in (int(x) for x in os.listdir("/proc") if x.isdigit()):
        try:
            env = open("/proc/%d/environ" % p, "rb").read()
        except OSError:
            continue
        if ("XDG_RUNTIME_DIR=%s\0" % rt).encode() in env:
            found.append(p)
    return found


host = Host()
before = host.real_layout()
for rt in glob.glob("/tmp/ofe2e.*"):
    pids = holders(rt)
    for sig in (signal.SIGTERM, signal.SIGKILL):
        for p in pids:
            try:
                os.kill(p, sig)
            except ProcessLookupError:
                pass
        time.sleep(0.5)
    shutil.rmtree(rt, ignore_errors=True)
    say("removed", rt, "and ended", len(pids), "processes")
for d in glob.glob("/tmp/ofe2e-state.*") + glob.glob("/tmp/ofe2e-data.*"):
    shutil.rmtree(d, ignore_errors=True)
    say("removed", d)
if any(m["name"] == OUTPUT for m in host.monitors()):
    host.hypr("output", "remove", OUTPUT)
    say("removed the virtual output", OUTPUT)
time.sleep(1)
say("real monitors unchanged:", host.real_layout() == before)
