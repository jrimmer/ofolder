import json, os, subprocess, sys, time
exec(open(os.path.join(os.path.dirname(os.path.abspath(__file__)), "strata_probe2.py")).read().split("d = dialog()")[0])  # reuse helpers only
OUT = os.path.join(HERE, "runs", "strata3"); os.makedirs(OUT, exist_ok=True)
call = subprocess.Popen([sys.executable, os.path.join(HERE, "portal_call.py"), "strata", HOME, "Strata spike save"], stdout=open(os.path.join(OUT, "call.out"), "w"), stderr=subprocess.STDOUT)
for _ in range(160):
    if dialog(): break
    time.sleep(0.05)
time.sleep(1.5); d = dialog(); A = '"address:%s"' % d["address"]
subprocess.run(["hyprctl", "dispatch", 'hl.dsp.window.float({ action = "on", window = %s })' % A], capture_output=True)
subprocess.run(["hyprctl", "dispatch", "hl.dsp.window.resize({ x = 1000, y = 640, relative = false, window = %s })" % A], capture_output=True)
time.sleep(0.8)
def header():
    nodes = dfs(app())
    i = next(k for k, n in enumerate(nodes) if n.get_role_name() == "button" and n.get_name() == "Copy path")
    out = []
    for n in nodes[max(0, i - 8):i + 2]:
        txt = ""
        try: txt = Atspi.Text.get_text(n, 0, min(Atspi.Text.get_character_count(n), 100))
        except Exception: pass
        out.append((n.get_role_name(), n.get_name(), txt))
    return out
say("HOME header:", header())
click("Documents"); time.sleep(0.8)
say("DOCUMENTS header:", header())
click("Downloads"); time.sleep(0.8)
say("DOWNLOADS header:", header())
nodes = dfs(app())
say("breadcrumb/list names:", [(n.get_role_name(), n.get_name()) for n in nodes if n.get_role_name() in ("list", "grouping") and n.get_name() and n.get_name() not in ("PINNED",)][:6])
click("Cancel file selection (Escape)"); time.sleep(1)
try: call.wait(5)
except subprocess.TimeoutExpired: call.kill()
say("left:", 1 if dialog() else 0)
