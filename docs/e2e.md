# End-to-end tests

`bin/e2e` runs ofolder against real dialogs without touching the desktop you are working in. The unit
tests and `bin/check` need no display; this does, so it is a separate command.

## What runs

For each of three backends (Flea 0.4.0 as installed, Flea 0.3.12 unpacked from a pinned release, and the
GTK portal) the suite starts its own copy of the stack inside a private Hyprland:

```
nested Hyprland (own sockets, own clipboard, own keys)  <- drawn on a virtual output of the real session
  private D-Bus  ->  the real xdg-desktop-portal  ->  ofolder portal  ->  the real Flea or GTK backend
  a small GTK4 program (e2e/app.py) that asks for a Save dialog, as an application would
```

The portal files and the `FileChooser=` chain live in the private home, so your `~/.config` is neither
read nor written. Keys reach a dialog only through `hyprctl dispatch send_shortcut` aimed at that
dialog's own window, never through whatever has the keyboard (`wtype` did not reach the GTK dialog). Folders are scratch folders under `/tmp`.

## What it proves (88 checks, about eight minutes)

Per backend (each of the 29 below runs on Flea 0.4, Flea 0.3.12 and GTK; Flea 0.4 has one more):

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

Flea 0.4 also checks its graceful `fleapicker.quit`. Moves are confirmed by the dialog itself first
(Flea's own `path`), then by the rails.

Clicks go to the verified window at a position inside it (`Session.click`), moving the pointer in steps
first. The positions of controls in the settings window and in a favorite's row menu are fixed numbers
read from screenshots; if the layout changes they must change too. `E2E_SHOTS=<folder>` keeps a
picture of a menu during a test.

## What it touches on the real session

One thing: a virtual output named `ofe2e-1`. Hyprland draws a window only on a monitor that is drawing,
and Flea's picker will not answer until it has drawn, so a hidden window cannot be tested. The output is
created with `hyprctl output create headless`, left where Hyprland puts it (to the right of your screen,
so a pointer pushed past the right edge can enter it while a run is going), and removed on every exit
path, including a crash and a signal. After removing it the suite checks your real monitors are
exactly as they were. Never give the output a position by hand: doing so once moved the real monitor.

`bin/e2e --cleanup` removes the output, the nested compositor and the scratch folders a killed run left.

## Needs

Hyprland with `hyprctl output create headless`, `wtype`, `wl-clipboard`, `busctl`, `dbus-daemon`,
Python 3 with `gi` (GTK 4), `/usr/lib/xdg-desktop-portal`, Flea, and `xdg-desktop-portal-gtk`. A missing
backend skips its flavour. Flea 0.3.12 is downloaded once to `~/.cache/ofolder-e2e` and checked against
a pinned SHA-256 (`e2e/flea03.py`); it needs `/usr/share/omarchy/shell/{Commons,Ui}` for Quickshell.

## Known limits

- The nested compositor is the Hyprland on disk. After a Hyprland update that has not been logged into
  yet, the tests run a newer one than the session you are using.
- Not covered: Strata; the search below a folder (the Search button and its results); the Shortcuts pane
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
- Copy path ran `wl-copy` with no type, so `wl-copy` ran the `xdg-mime` shell script on the text and the
  strip waited for it on its UI thread. The type is now named. Failures that looked like a command being
  ignored stopped after this change (five full runs in a row, against four failures in eleven before),
  but the cause of those failures was not isolated, so this is a likely fix and not a proved one.
- The keys unit tests left nine folders in `/tmp` per run. They remove themselves now.

## Side effect to know about

The compositor's own portal backend (`xdg-desktop-portal-hyprland`) crashes inside a nested session. Until
the suite learned to keep it from starting, every run left a core dump (73 of them on 2026-11-03, all
from test runs) and may have raised a crash notification. A private `hyprland.portal` now hides it; a
run leaves no dump. The old dumps are still in `coredumpctl`.

## Open

Once, while the rails were being polled several times a second, the strip's main thread sat inside
Mesa's Wayland surface creation (waiting on a roundtrip to the compositor) at near 100% CPU and the
rails stopped updating. A core showed where; it did not recur after the polling was made gentler, and
it has not been seen outside the nested session. Not understood.

`OFOLDER_DEBUG=1` makes the rails log to stderr what they receive and ignore. The suite sets it, and
a failing move prints those lines with the process list.
