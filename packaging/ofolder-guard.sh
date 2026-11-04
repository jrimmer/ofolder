#!/bin/sh
# Keeps file dialogs working, and ofolder in front of them, without anyone having to notice.
# It is plain shell on purpose: it must still run when the ofolder program is gone.
#
#  * ofolder is missing, will not run `ofolder version`, or fails `ofolder portal --check` with status 3:
#    its portal file is set aside, so the portal skips ofolder and the next backend (Flea, GTK) answers.
#    When ofolder is back the file is put back. The portal frontend is restarted after either, so it
#    takes notice. Any other status from `--check` is not a verdict: an older build does not know the
#    option and exits 1, and status 4 means the check could not be made; both keep their portal file.
#    A status 3 is confirmed by asking again twice before anything is moved.
#  * ofolder is there but another program took it out of the file-chooser chain: `ofolder repair`
#    puts it back (a copy of the file is kept; `ofolder repair --undo` goes back).
#
# `ofolder repair --watch on` installs the systemd units that run this when the program or the
# portal configuration changes, and once at login. Nothing else runs it.
set -u
# Started by systemd, this may not have been given the desktop name, which ofolder needs to find
# the portal configuration the frontend reads; the user manager has it.
if [ -z "${XDG_CURRENT_DESKTOP:-}" ] && [ -z "${OFOLDER_NO_SYSTEMD:-}" ]; then
  XDG_CURRENT_DESKTOP=$(systemctl --user show-environment 2>/dev/null | sed -n 's/^XDG_CURRENT_DESKTOP=//p')
  export XDG_CURRENT_DESKTOP
fi
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
  [ -n "${OFOLDER_NO_SYSTEMD:-}" ] && return 0
  systemctl --user try-restart xdg-desktop-portal.service >/dev/null 2>&1 || true
}

exe=$(sed -n 's/^Exec=\([^ ]*\).*/\1/p' "$service" 2>/dev/null | head -n 1)

# A program that is there but cannot start (a bad copy, a missing library) would stop every file
# dialog as surely as one that is gone, so it has to answer `version` to count as there.
state=""
if [ -z "$exe" ] || [ ! -x "$exe" ] || ! "$exe" version >/dev/null 2>&1; then
  state="is missing or will not start"
else
  # A build can answer `version` and still fail once it serves the portal. `portal --check` runs the
  # portal's start-up without claiming its name, and only status 3 says that it failed.
  # Status 4 means the check could not be made (the session bus was not there to ask), which says
  # nothing about the build. Even status 3 is asked again twice, a few seconds apart, so a hiccup
  # that lands on one asking cannot take ofolder out of the dialogs.
  check() { "$exe" portal --check >/dev/null 2>&1; return $?; }
  check
  status=$?
  for wait in ${OFOLDER_GUARD_RETRY_WAITS-2 5}; do
    [ "$status" -eq 3 ] || break
    sleep "$wait"
    check
    status=$?
  done
  [ "$status" -eq 3 ] && state="starts but fails its portal check"
fi
if [ -n "$state" ]; then
  if [ -f "$portal" ]; then
    mkdir -p "$(dirname "$held")"
    mv "$portal" "$held"
    restart
    say "ofolder $state${exe:+ ($exe)}, so file dialogs are using the next backend. Put a working one back and they return."
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
result=$("$exe" repair --from-guard $flags 2>&1) || exit 0
case "$result" in
  *"put ofolder first"*) say "another program had taken ofolder out of the file-chooser chain; it is back first. To undo: ofolder repair --undo (the guard then leaves the chain alone until you run ofolder repair)" ;;
esac
exit 0
