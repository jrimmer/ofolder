#!/usr/bin/env bash
# ofolder one-line installer. Read this before piping it to bash:
#
#   curl -fsSL https://github.com/jrimmer/ofolder/releases/latest/download/install.sh | bash
#
# Flags (pass them through bash -s --):
#
#   --no-setup    install the files only; don't register ofolder as the file-chooser
#                 backend afterwards
#   --uninstall   undo the setup, then remove ofolder
#   --purge       with --uninstall, also remove your settings, favorites and recents
#   --help        show this help
#
# It installs the release tarball under ~/.local, no root needed: the program in
# ~/.local/bin/ofolder (the one it replaces is kept as ofolder.previous) and the
# update tool in ~/.local/lib/ofolder/praeco. Linux x86_64 only.
# Files come from $OFOLDER_RELEASE_URL (the GitHub "latest/download" URL by
# default) and each one is checked against its .sha256 before use.
#
# A fresh install then runs `ofolder repair`, which changes how every file dialog
# on this machine is requested; the script says so first and how to undo it.

set -euo pipefail

RELEASE_URL="${OFOLDER_RELEASE_URL:-https://github.com/jrimmer/ofolder/releases/latest/download}"
ARCHIVE="ofolder-linux-x86_64.tar.gz"

usage() {
    cat <<'USAGE'
ofolder one-line installer. Read this before piping it to bash:

  curl -fsSL https://github.com/jrimmer/ofolder/releases/latest/download/install.sh | bash

Flags (pass them through bash -s --):

  --no-setup    install the files only; don't register ofolder as the file-chooser
                backend afterwards
  --uninstall   undo the setup, then remove ofolder
  --purge       with --uninstall, also remove your settings, favorites and recents
  --help        show this help

It installs the release tarball under ~/.local, no root needed: the program in
~/.local/bin/ofolder (the one it replaces is kept as ofolder.previous) and the
update tool in ~/.local/lib/ofolder/praeco. Linux x86_64 only.
Files come from $OFOLDER_RELEASE_URL (the GitHub "latest/download" URL by
default) and each one is checked against its .sha256 before use.

A fresh install then runs `ofolder repair`, which changes how every file dialog
on this machine is requested; the script says so first and how to undo it.
USAGE
}

no_setup=false
uninstall=false
purge=false

for arg in "$@"; do
    case "$arg" in
        --no-setup) no_setup=true ;;
        --uninstall) uninstall=true ;;
        --purge) purge=true ;;
        --help|-h) usage; exit 0 ;;
        *) echo "install.sh: unknown option $arg (try --help)" >&2; exit 2 ;;
    esac
done

if $purge && ! $uninstall; then
    echo "install.sh: --purge only goes with --uninstall (try --help)" >&2
    exit 2
fi

if [ -z "${HOME:-}" ] || [ ! -d "$HOME" ]; then
    echo "install.sh: HOME is not set to a folder." >&2
    exit 1
fi

bindir="$HOME/.local/bin"
libdir="$HOME/.local/lib/ofolder"

# --- uninstall ------------------------------------------------------------

if $uninstall; then
    if [ -x "$bindir/ofolder" ]; then
        args=(uninstall)
        $purge && args+=(--purge)
        status=0
        "$bindir/ofolder" "${args[@]}" || status=$?
        case "$status" in
            0) ;;
            126|127)
                # The program cannot start at all, so there is nothing to ask. The guard, if it
                # is on, sets the portal file aside by itself when the program is gone.
                echo "install.sh: $bindir/ofolder will not run, so its setup could not be undone by it." >&2
                echo 'Remove ~/.local/share/xdg-desktop-portal/portals/ofolder.portal, and the ofolder entry in the' >&2
                echo 'portals.conf file under ~/.config/xdg-desktop-portal, by hand if dialogs misbehave.' >&2
                ;;
            *)
                # Taking the program away now would leave the setup pointing at nothing.
                echo "install.sh: 'ofolder uninstall' failed (exit $status); nothing was removed." >&2
                echo "Fix what it reports and run this again." >&2
                exit 1
                ;;
        esac
    fi
    rm -f "$bindir/ofolder" "$bindir/ofolder.previous"
    rm -rf "$libdir"
    rmdir "$HOME/.local/lib" 2>/dev/null || true
    echo "ofolder is uninstalled."
    exit 0
fi

# --- install --------------------------------------------------------------

# Work in a private directory that goes away even if we fail.
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

if command -v curl >/dev/null 2>&1; then
    fetch() { curl -fsSL "$1" -o "$2"; }
elif command -v wget >/dev/null 2>&1; then
    fetch() { wget -q -O "$2" "$1"; }
else
    echo "install.sh: need curl or wget to download ofolder." >&2
    exit 1
fi

if ! command -v sha256sum >/dev/null 2>&1; then
    echo "install.sh: need sha256sum to check the download." >&2
    exit 1
fi

os=$(uname -s)
machine=$(uname -m)
if [ "$os" != Linux ] || [ "$machine" != x86_64 ]; then
    echo "install.sh: ofolder is released for Linux on x86_64 only, and this is $os on $machine." >&2
    echo "It docks beside file dialogs on Hyprland, so there is nothing to install elsewhere." >&2
    exit 1
fi

# Download an asset and its .sha256, then verify before anyone uses it. A
# mismatch aborts the whole run, so a half-installed tree never happens.
download() {
    local name="$1" dest="$tmp/$1" want got
    if ! fetch "$RELEASE_URL/$name" "$dest"; then
        echo "install.sh: could not download $name from $RELEASE_URL" >&2
        exit 1
    fi
    if ! fetch "$RELEASE_URL/$name.sha256" "$dest.sha256"; then
        echo "install.sh: could not download $name.sha256" >&2
        exit 1
    fi
    want=$(awk 'NR == 1 { print $1 }' "$dest.sha256")
    case "$want" in
        *[!0-9a-f]*|"") want="" ;;
    esac
    if [ "${#want}" -ne 64 ]; then
        echo "install.sh: $name.sha256 does not hold a SHA-256; nothing was installed." >&2
        exit 1
    fi
    got=$(sha256sum "$dest" | awk '{ print $1 }')
    if [ "$got" != "$want" ]; then
        echo "install.sh: checksum mismatch for $name; nothing was installed." >&2
        exit 1
    fi
    printf '%s' "$dest"
}

# Keep the path in a variable: `exit 1` inside a command substitution only leaves
# the subshell, and we want the failure to stop the whole script.
archive=$(download "$ARCHIVE")

# The tarball holds ofolder-<version>/bin/ofolder and ofolder-<version>/lib/ofolder/praeco and
# nothing else. Look at every entry before extracting any, so an archive that reaches outside its
# own folder is refused.
listing=$(tar -tzf "$archive")
top=${listing%%/*}
if ! [[ "$top" =~ ^ofolder-[A-Za-z0-9._+-]+$ ]]; then
    echo "install.sh: the archive's top folder '$top' is not ofolder-<version>; nothing was installed." >&2
    exit 1
fi
version=${top#ofolder-}
while IFS= read -r entry; do
    case "$entry" in
        "$top"|"$top"/*) ;;
        *) echo "install.sh: the archive holds '$entry' outside $top/; nothing was installed." >&2; exit 1 ;;
    esac
    case "/$entry/" in
        */../*) echo "install.sh: the archive holds '$entry'; nothing was installed." >&2; exit 1 ;;
    esac
done <<<"$listing"

stage="$tmp/stage"
mkdir "$stage"
tar -xzf "$archive" -C "$stage" --no-same-owner
for f in bin/ofolder lib/ofolder/praeco; do
    if [ ! -f "$stage/$top/$f" ] || [ -L "$stage/$top/$f" ]; then
        echo "install.sh: the archive has no $f; nothing was installed." >&2
        exit 1
    fi
done

# The installed version, or empty for a fresh install.
have=""
fresh=true
if [ -e "$bindir/ofolder" ]; then
    fresh=false
    have=$("$bindir/ofolder" version 2>/dev/null | awk 'NR == 1 { print $2 }') || have=""
fi

if [ "$have" = "$version" ]; then
    echo "ofolder $version is already installed."
    exit 0
fi

if $fresh; then
    echo "Installing ofolder $version..."
elif [ -n "$have" ]; then
    echo "Updating ofolder $have -> $version..."
else
    echo "Replacing ofolder with $version..."
fi

mkdir -p "$bindir" "$libdir"

# Each file is written next to its place and then renamed over it, so a program that is running
# (the portal, usually) keeps the file it started from and nothing is ever half written. The
# program being replaced is kept as ofolder.previous.
if [ -e "$bindir/ofolder" ]; then
    ln -f "$bindir/ofolder" "$bindir/.ofolder.previous.new"
    mv -f "$bindir/.ofolder.previous.new" "$bindir/ofolder.previous"
fi
install -m755 "$stage/$top/bin/ofolder" "$bindir/.ofolder.new"
mv -f "$bindir/.ofolder.new" "$bindir/ofolder"
install -m755 "$stage/$top/lib/ofolder/praeco" "$libdir/.praeco.new"
mv -f "$libdir/.praeco.new" "$libdir/praeco"

echo "Installed $bindir/ofolder and $libdir/praeco."
case ":$PATH:" in
    *":$bindir:"*) ;;
    *) echo "Note: $bindir is not on your PATH; add it to run 'ofolder' directly." >&2 ;;
esac

# ofolder drives the dialogs through these; it does not install them.
missing=()
for tool in hyprctl wtype wl-copy; do
    command -v "$tool" >/dev/null 2>&1 || missing+=("$tool")
done
if [ "${#missing[@]}" -gt 0 ]; then
    echo "Warning: ofolder needs these, and they are not installed: ${missing[*]} (this script does not install them)." >&2
    echo "On Arch: sudo pacman -S hyprland wtype wl-clipboard" >&2
fi

# An update keeps the setup it finds; only a fresh install sets up.
if ! $fresh; then
    echo "ofolder $version is installed; your setup was left as it was."
    echo "The previous program is kept as $bindir/ofolder.previous."
    exit 0
fi

if $no_setup; then
    echo "Skipped the setup (--no-setup). When you want it: ofolder repair"
    exit 0
fi

cat <<'SETUP'

Next is the setup. It changes how every file dialog on this machine is requested:
  - it registers ofolder as a file-chooser backend (two small files under ~/.local/share),
  - it puts ofolder first in your portal configuration in ~/.config/xdg-desktop-portal/
    (a copy of the file as it was is kept),
  - it installs a guard (two systemd user units) that keeps ofolder in that list, and
  - it restarts the portal frontend, which is what hands out the dialogs.
Dialogs still open as before: ofolder passes each one on to your usual file chooser.

SETUP
if "$bindir/ofolder" repair; then
    echo
    "$bindir/ofolder" doctor || echo "Doctor found something to fix; the lines above say what and how."
    echo
    echo "To undo:"
    echo "  ofolder pause            dialogs pass straight through, nothing is removed"
    echo "  ofolder repair --undo    put your portal configuration back as it was"
    echo "  install.sh --uninstall   undo the setup and remove ofolder (curl -fsSL \"$RELEASE_URL/install.sh\" | bash -s -- --uninstall)"
else
    echo "The install is done. The setup failed; retry with:" >&2
    echo "    ofolder repair" >&2
fi
