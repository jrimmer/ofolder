#!/usr/bin/env bash
# Probe: where could the strip read the picker's current folder from?
export HYPRLAND_INSTANCE_SIGNATURE=efb50993780079460b0cbed1363e2166a2de1d9f_1792084656_1809977280 WAYLAND_DISPLAY=wayland-1
OUT=$1; mkdir -p "$OUT"
REQ='{"mode":"save","title":"Probe","app":"probe","accept":"","multiple":false,"directory":false,"folder":"'$HOME'","file":"","name":"p.txt","files":[],"filters":[],"current":"","currentIndex":-1}'
FLEA_PICKER="$REQ" flea --pick "$OUT/reply.json" >"$OUT/flea.out" 2>&1 &
FP=$!
sleep 3
# every process in the picker's tree
PIDS="$FP $(pgrep -P $FP) $(pgrep -f 'fleapicker|boot/picker.qml')"
PIDS=$(echo $PIDS | tr ' ' '\n' | sort -un | tr '\n' ' ')
echo "pids: $PIDS" > "$OUT/tree.txt"; for p in $PIDS; do echo "$p $(tr '\0' ' ' < /proc/$p/cmdline | head -c 120)" >> "$OUT/tree.txt"; done
snap() {
  tag=$1
  md5sum ~/.local/state/flea/ui.json > "$OUT/$tag.uijson" 2>&1
  stat -c '%y' ~/.local/state/flea/ui.json >> "$OUT/$tag.uijson"
  for p in $PIDS; do
    echo "== $p cwd=$(readlink /proc/$p/cwd)" >> "$OUT/$tag.fds"
    ls -l /proc/$p/fd 2>/dev/null | grep -E 'anon_inode:inotify|/home|/tmp' | awk '{print $9,$10,$11}' >> "$OUT/$tag.fds"
    grep -h '^inotify' /proc/$p/fdinfo/* 2>/dev/null | sed 's/^/   /' >> "$OUT/$tag.fds"
  done
}
snap A
hyprctl dispatch 'hl.dsp.focus({ window = "class:com.thisisgm.flea.picker" })' >/dev/null 2>&1; sleep 0.3
wtype -M ctrl l -m ctrl; sleep 0.3; wtype "$HOME/Documents"; sleep 0.2; wtype -k Return; sleep 1.5
snap B
wtype -k Escape; sleep 1; wait $FP 2>/dev/null
echo "reply: $(cat $OUT/reply.json 2>/dev/null)" >> "$OUT/tree.txt"
