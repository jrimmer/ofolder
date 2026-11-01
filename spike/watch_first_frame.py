import json, subprocess, sys, time
end = time.time() + 14
seen = []
while time.time() < end:
    out = subprocess.run(["hyprctl", "clients", "-j"], capture_output=True, text=True).stdout
    try:
        cs = json.loads(out)
    except json.JSONDecodeError:
        continue
    for c in cs:
        if c["class"] == "spike.strip":
            key = (c["floating"], tuple(c["at"]), tuple(c["size"]))
            if not seen or seen[-1][1] != key:
                seen.append((round(time.time() % 1000, 2), key))
    time.sleep(0.015)
for t, (fl, at, sz) in seen:
    print("strip state t=%s floating=%s at=%s size=%s" % (t, fl, list(at), list(sz)))
