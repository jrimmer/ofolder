#!/bin/sh
# Keeps file dialogs working, and ofolder in front of them, without anyone having to notice.
# It is plain shell on purpose: it must still run when the ofolder program is gone.
#
#  * ofolder is missing, or will not run `ofolder version`: its portal file is set aside, so the portal skips ofolder
#    and the next backend (Flea, GTK) answers. When ofolder is back the file is put back. The portal
#    frontend is restarted after either, so it takes notice.
#  * ofolder is there but another program took it out of the file-chooser chain: `ofolder repair`
#    puts it back (a copy of the file is kept; `ofolder repair --undo` goes back).
#
# `ofolder repair --watch on` installs the systemd units that run this when the program or the
# portal configuration changes, and once at login. Nothing else runs it.
set -u
data="${XDG_DATA_HOME:-$HOME/.local/share}"
service="$data/dbus-1/services/org.freedesktop.impl.portal.desktop.ofolder.service"
portal="$data/xdg-desktop-portal/portals/ofolder.portal"
held="$data/ofolder/ofolder.portal.held"

say() {
  echo "ofolder-guard: $*"
  if command -v notify-send >/dev/null 2>&1 && [ -z "${OFOLDER_GUARD_QUIET:-}" ]; then
    notify-send -a ofolder "ofolder" "$*" >/dev/null 2>&1 || true
  fi
}

# A portal frontend that started while ofolder was broken has dropped the file-chooser interface
# altogether, so after setting the file aside, or putting it back, the frontend is restarted.
restart() {
  [ -n "${OFOLDER_GUARD_NO_RESTART:-}" ] && return 0
  systemctl --user try-restart xdg-desktop-portal.service >/dev/null 2>&1 || true
}

exe=$(sed -n 's/^Exec=\([^ ]*\).*/\1/p' "$service" 2>/dev/null | head -n 1)

# A program that is there but cannot start (a bad copy, a missing library) would stop every file
# dialog as surely as one that is gone, so it has to answer `version` to count as there.
if [ -z "$exe" ] || [ ! -x "$exe" ] || ! "$exe" version >/dev/null 2>&1; then
  if [ -f "$portal" ]; then
    mkdir -p "$(dirname "$held")"
    mv "$portal" "$held"
    restart
    say "ofolder is missing or will not start${exe:+ ($exe)}, so file dialogs are using the next backend. Put a working one back and they return."
  fi
  exit 0
fi

if [ -f "$held" ] && [ ! -f "$portal" ]; then
  mkdir -p "$(dirname "$portal")"
  mv "$held" "$portal"
  restart
  say "ofolder is back: file dialogs have it again."
fi

flags=""
[ -n "${OFOLDER_GUARD_NO_RESTART:-}" ] && flags="--no-restart"
# shellcheck disable=SC2086
result=$("$exe" repair $flags 2>&1) || exit 0
case "$result" in
  *"put ofolder first"*) say "another program had taken ofolder out of the file-chooser chain; it is back first. To undo: ofolder repair --undo" ;;
esac
exit 0
