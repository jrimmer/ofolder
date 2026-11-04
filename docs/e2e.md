# End-to-end tests

`bin/e2e` runs ofolder against real dialogs without touching the desktop you are working in. The unit
tests and `bin/check` need no display; this does, so it is a separate command.

## What runs

For each of four backends (Flea 0.4.0 as installed, Flea 0.3.12 unpacked from a pinned release, the
GTK portal, and Strata 1.1.1 as installed) the suite starts its own copy of the stack inside a private Hyprland:

```
nested Hyprland (own sockets, own clipboard, own keys)  <- drawn on a virtual output of the real session
  private D-Bus  ->  the real xdg-desktop-portal  ->  ofolder portal  ->  the real Flea or GTK backend
  a small GTK4 program (e2e/app.py) that asks for a Save dialog, as an application would
```

The portal files and the `FileChooser=` chain live in the private home, so your `~/.config` is neither
read nor written. Keys reach a dialog only through `hyprctl dispatch send_shortcut` aimed at that
dialog's own window, never through whatever has the keyboard (`wtype` did not reach the GTK dialog). Folders are scratch folders under `/tmp`.

## What it proves (about 160 checks, about ten minutes)

Per backend (each of the 29 below, and the real-app shapes and the idle-CPU check after them, runs on Flea 0.4, Flea 0.3.12, GTK and Strata; Flea 0.4 has one more):

- Docking and moving: the rails dock either side of the dialog and do not take the keyboard; they follow
  a moved dialog; the folder reads back (copy path); parent and back move the dialog and read back (for
  Flea the dialog itself is asked as a second witness); forty quick presses do not bring the rails down.
- Ending a dialog: cancel leaves no windows or key bindings and tells the application "cancelled";
  confirm returns the chosen file; a confirmed dialog is remembered and a cancelled one is not; pause
  leaves the dialog untouched.
- Favorites: the folder is added and taken off again; a favorite chosen from its menu takes the dialog
  there; from a row's own menu a favorite is removed, moved down and renamed.
- Menus, chosen with a real click: recent folders, recent files (to the file's folder) and places (home).
- Making and finding: a new folder is created from its prompt and the dialog goes into it; find ranks
  the folders it knows and Enter goes to the best.
- Settings window: it opens and closes; a general switch is saved, the recents count is stepped down and
  saved, the preview size cap is chosen, a rail icon is hidden and the default layout restores it, each
  checked in `settings.toml`.
- Preview: it shows under the dialog while an image is selected, is hidden by its shortcut, and does not
  show for a text file.
- Shortcuts: one set in the settings is bound while the dialog is open and is gone after it closes.
- `ofolder status` reports the file-chooser chain and warns when ofolder is not in it.
- Real-app shapes: several files chosen in an Open dialog; a folder chosen in a folder dialog; a dialog
  that offers only `*.txt`; saving over an existing file (the dialog's own Replace prompt, then the file
  returned and left untouched); a folder named with spaces, quotes, `&`, `;`, `$` and Japanese; a path 14
  levels deep; a folder reached through a link; a dialog given no start folder.
- Idle CPU: the rails use under 25% of a core when nothing is happening. Judged on the release build
  only (`bin/e2e --release`); the debug build uses far more and is skipped.

Flea 0.4 also checks its graceful `fleapicker.quit`. Moves are confirmed by the dialog itself first
(Flea's own `path`), then by the rails.

A nested compositor offers its clients a pointer only when the real seat has one, so with no mouse
attached (a dock, a remote session) no click would land. `e2e/vpointer.py` makes the nested session a
virtual pointer of its own (the wlroots virtual pointer protocol, spoken directly), so the suite does not
depend on the machine it runs on having one.

Clicks go to the verified window at a position inside it (`Session.click`), moving the pointer in steps
first. The positions of controls in the settings window and in a favorite's row menu are fixed numbers
read from screenshots; if the layout changes they must change too. `E2E_SHOTS=<folder>` keeps a
picture of a menu during a test.

The same virtual pointer turns the mouse wheel (`Session.scroll`, through a named pipe it reads), which
the settings window needs: its General pane scrolls, and the Updates block is at the bottom. A wheel
turned far past the end stops there, so the block is always in the same place.

Updates are tested on GTK only, since nothing in them depends on the backend: the stand-in praeco from
`tests/support` is set in the session's environment (`OFOLDER_PRAECO`, with the running version
pretended to be 0.0.1), a state file is written as a check would write it, and real clicks follow.
It proves: the switch is saved; Install runs praeco detached with the progress file and the portal's own
pid to stop, and Later, Skip and an install praeco cannot do change or start nothing they should not; the
omarchy registration asks first, keeping it off changes nothing, Escape keeps the window open, turning
it on runs `praeco init` and off runs it with `--remove`. `UpdateCheck` starts a portal whose check is
due at once and proves the answer is recorded privately (0600), that praeco is asked with the running
version and `--json`, and that a failed check writes nothing.

## What it touches on the real session

One thing: a virtual output named `ofe2e-1`. Hyprland draws a window only on a monitor that is drawing,
and Flea's picker will not answer until it has drawn, so a hidden window cannot be tested. The output is
created with `hyprctl output create headless`, left where Hyprland puts it (to the right of your screen,
so a pointer pushed past the right edge can enter it while a run is going), and removed on every exit
path, including a crash and a signal. After removing it the suite checks your real monitors are
exactly as they were. Never give the output a position by hand: doing so once moved the real monitor.

`bin/e2e --cleanup` removes the output, the nested compositor and the scratch folders a killed run left.

## Reliability and recovery

`FailOpen` and `Recovery` check what happens when things go wrong, in the private session:

- stopping the rails leaves the dialog working; a chain that names a backend since uninstalled falls through to
  the next installed one (it used to forward to nothing and the dialog failed);
- `doctor` flags a chain that lost ofolder, `repair` puts it back with a copy kept, the rails return, a second
  `repair` changes nothing and `--undo` restores the file as it was; `repair` recreates a missing registration;
- ofolder deleted, with the guard run, the dialog still opens, whether it was deleted while the portal ran or was
  already gone when the portal started (the guard restarts the portal in that case).

Two known limits are recorded as expected failures, not hidden: with ofolder missing, or there but dead,
and no guard, every file dialog fails, because the portal asks a program that does nothing. The guard covers
both: it sets the portal file aside when the program is missing or will not answer `ofolder version` (a bad
build, a missing library). What it cannot see is a build that answers `version` and then fails inside the
portal itself, so `bin/install` (below) is the other half: it checks the new build runs, keeps the previous
one and can put it back. `test_guard.py` runs the guard script and `repair --watch` in a temporary home, with
no display, and is part of `bin/check`.

## Installing safely

`bin/install` builds the release build, checks the new build starts and answers, keeps the build it replaces
as `~/.local/bin/ofolder.previous`, swaps in the new one atomically and ends the old portal process.
`bin/install --rollback` puts the previous build back. After installing, open one real Save or Open dialog;
if dialogs stop appearing, roll back (`ofolder pause` also makes dialogs pass through untouched).

## Release build and the canary

`bin/e2e --release` runs everything against the release build, the one that is installed. `bin/canary`
(`e2e/canary.py`) records the versions of Flea, Strata, the portal frontend, the GTK backend and Hyprland,
installed and running, and the state of the chain, in `~/.local/state/ofolder/canary.json`. It prints what
changed since the last look, warns when ofolder is not leading the chain or when the installed Hyprland is not
the one running, and with `--run` runs `bin/e2e --release` if anything changed. It runs only when you run it.

## Strata

Strata's `--install-portal` restarts the real session's portal frontend, so the suite does not run it.
It writes the two files that command would (`strata.portal` and the D-Bus service running
`strata --portal`) into the private data directory instead. Strata's dialog opens tiled; ofolder floats it
with a rule before it opens, as it does on the desktop.

## Needs

Hyprland with `hyprctl output create headless`, `wtype`, `wl-clipboard`, `busctl`, `dbus-daemon`,
Python 3 with `gi` (GTK 4), `/usr/lib/xdg-desktop-portal`, Flea, and `xdg-desktop-portal-gtk`. A missing
backend skips its flavour. Flea 0.3.12 is downloaded once to `~/.cache/ofolder-e2e` and checked against
a pinned SHA-256 (`e2e/flea03.py`); it needs `/usr/share/omarchy/shell/{Commons,Ui}` for Quickshell.

## Known limits

- The nested compositor is the Hyprland on disk. After a Hyprland update that has not been logged into
  yet, the tests run a newer one than the session you are using.
- Two of the real-app shapes are skipped on Strata with the reason stated, because no way to drive them was
  found: confirming a folder in its folder dialog (Enter, Tab, Ctrl/Shift/Alt+Enter and a click all did
  nothing) and selecting several files (Ctrl+A, Shift+Down and Space selected one). They are gaps, not passes.
- Where a dialog does not act on a click sent to it (Flea 0.3's Qt window), its buttons are reached by
  keyboard instead; the keys are per backend in `test_portal.py`.
- Not covered: the search below a folder (the Search button and its results); the Shortcuts pane
  that records a key combination; the Rails pane beyond hiding an icon and resetting; the Messages pane;
  file manager and terminal launching; theme changes; the preview of a video.
- A configured shortcut cannot be pressed: a virtual keyboard (`wtype`) does not trigger any bind in the
  nested compositor, not even a plain one, so only the binding itself is checked.
- The first Return into a fresh Flea 0.4 Save dialog is ignored and the second confirms it. Measured
  twice with ofolder paused and twice with the rails up, always two presses, so it is Flea's and not
  ofolder's. The suite repeats the press, one at a time, each sent to the dialog's own window.
- Nothing here is part of `bin/check`.

## Bugs it found when first run

- A rail command repeated quickly made the rails panic (`cannot update Shared while it is already being
  updated`): a superseded menu, tooltip, settings or preview window was removed inside the update that
  superseded it. The removal now waits until that update ends.
- Under Flea 0.3.12 with no locale set (as in a portal started by D-Bus), `qs ipc` logs a warning before
  its answer, ofolder read the snapshot as nothing and refused every move. The answer is now taken from
  the first line that opens an object.
- Strata checks a typed path against a folder listing that lags a folder made a moment ago, so entering
  a new folder at once left its path bar open and the dialog where it was (two failures in three). The
  adapter now asks for a second's pause after the folder is made; five of five then passed.
- Copy path ran `wl-copy` with no type, so `wl-copy` ran the `xdg-mime` shell script on the text and the
  strip waited for it on its UI thread. The type is now named. Failures that looked like a command being
  ignored stopped after this change (five full runs in a row, against four failures in eleven before),
  but the cause of those failures was not isolated, so this is a likely fix and not a proved one.
- `wtype` decodes its arguments with the process locale and failed on any non-ASCII character when there
  was none (a portal started by D-Bus may have none), so folders with accents or Japanese names could not
  be entered. It now runs under `C.UTF-8`.
- The keys unit tests left nine folders in `/tmp` per run. They remove themselves now.

## Side effect to know about

The compositor's own portal backend (`xdg-desktop-portal-hyprland`) crashes inside a nested session. Until
the suite learned to keep it from starting, every run left a core dump (73 of them on 2026-11-03, all
from test runs) and may have raised a crash notification. A private `hyprland.portal` now hides it; a
run leaves no dump. The old dumps are still in `coredumpctl`.

## Open

Once, while the rails were being polled several times a second, the strip's main thread sat inside
Mesa's Wayland surface creation (waiting on a roundtrip to the compositor) at near 100% CPU and the
rails stopped updating. It did not recur in 40 further runs of the same stress (20 on each of the debug
and release builds), and the release build idles at 3 to 5% of a core on the real desktop, against about
90% for the debug build. The likeliest cause is the debug build's CPU saturation, but that is not proved.

`OFOLDER_DEBUG=1` makes the rails log to stderr what they receive and ignore. The suite sets it, and
a failing move prints those lines with the process list.
