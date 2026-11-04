# Switching ofolder on

ofolder does nothing to your apps until it is registered as a file-chooser backend. This is not
done by the build or by any test, because it changes how every save and open dialog on the
machine is requested. These are the steps, to be done by hand, with the way back.

1. Install the binary where D-Bus can find it, for example `~/.local/bin/ofolder`.
2. Back up what is about to change:
   `cp -a ~/.config/xdg-desktop-portal ~/xdg-desktop-portal.backup`
3. Install the two files, replacing `@OFOLDER@` with the absolute path of the binary (a path with
   spaces, quotes or backslashes is not safe in a D-Bus service file):
   - `ofolder.portal` to `~/.local/share/xdg-desktop-portal/portals/ofolder.portal`
   - `org.freedesktop.impl.portal.desktop.ofolder.service` to
     `~/.local/share/dbus-1/services/org.freedesktop.impl.portal.desktop.ofolder.service`
4. Put ofolder first in the file-chooser preference, in the portal configuration file for your
   desktop (on this machine `~/.config/xdg-desktop-portal/hyprland-portals.conf`), keeping your
   existing backends after it. ofolder forwards each request to the backend after it in this list:

   ```
   org.freedesktop.impl.portal.FileChooser=ofolder;flea;gtk
   ```
5. Restart the portal frontend: `systemctl --user restart xdg-desktop-portal.service`.

ofolder starts on demand through D-Bus when the first dialog is requested, so nothing needs to be
started at login.

## Going back

Restore the backup, remove the two installed files, and restart the portal frontend. Or, without
touching anything else, run `ofolder pause`: every dialog then passes straight through, untouched.
`ofolder resume` turns it back on.

## Per-app features

The portal gives ofolder an application id only for sandboxed apps. Ordinary apps arrive with an
empty id, so ofolder falls back to the class of the window that had focus when the dialog was
requested, written `class:<name>`. A start-folder rule for an ordinary app is therefore written
with that prefix in `~/.config/ofolder/settings.toml`:

```
[start_folders]
"class:zen" = "~/Downloads"
```

## The settings file

`~/.config/ofolder/settings.toml` is read again for every dialog. The gear at the foot of the
right rail opens a settings window that writes the same file, keeping its comments, its order
and any key it does not know. The settings window's Apps and privacy pane edits `excluded_apps`,
`recents_excluded` and `start_folders` (removing one of the default folders asks first). A value of the wrong type is reported and ignored; the rest
still apply.

| Key | Meaning | Default |
|---|---|---|
| `keep_dialog_after_file_manager` | Keep the dialog open after "open in file manager" | `true` |
| `recents_enabled` | Remember recent folders and files | `true` |
| `recents_max` | How many recent items are kept, 1 to 500 | `20` |
| `recents_days` | Days a recent folder or file is kept after its last use; `0` keeps them for ever | `90` |
| `recents_excluded` | Folders, and everything below them, never recorded as recent. Naming your own list replaces the default one | `~/.ssh`, `~/.gnupg`, `~/.password-store`, `~/.local/share/keyrings`, `~/.config/Bitwarden`, `~/.config/1Password`, `~/.config/KeePassXC`, `~/.mozilla`, `~/.config/google-chrome`, `~/.config/chromium`, `~/.config/BraveSoftware`, `~/.local/share/Trash` |
| `excluded_apps` | Application ids whose dialogs are left alone | none |
| `file_manager`, `terminal` | Commands to use instead of the desktop's own; the folder is added at the end | detected |
| `start_folders` | A table of application id to the folder its dialogs start in, when the app gives none | none |
| `paused` | Set by `ofolder pause`: every dialog passes through untouched | `false` |
| `guard` | Keep ofolder in the file-chooser chain and dialogs working without it (see "Doing it for you"). Turning it off in the settings window asks first | `true` |
| `updates` | Look for a newer release of ofolder (needs praeco; see `docs/updates.md`) | `true` |
| `update_with_system` | Let `omarchy update` also update ofolder. Turning it on in the settings window asks first | `false` |
| `[rails]` `left`, `right` | The icons of each rail, top to bottom. `"-"` is a divider line, and an icon left out is hidden | as below |
| `[shortcuts]` | A table of command to key combination, such as `places = "SUPER+ALT+P"`. Set in the settings window's Shortcuts pane | none |
| `[preview]` `enabled` | Show a panel under the dialog while an image or video is selected | `true` |
| `[preview]` `video_thumbnails` | Show a still from a selected video, made with `ffmpeg` when it is installed | `true` |
| `[preview]` `max_mb` | Files larger than this many MB are described, not opened (1 to 4096) | `64` |

```
[rails]
left = ["back", "forward", "parent", "-", "favorites", "places", "recent_folders", "recent_files"]
right = ["new_folder", "find", "-", "favorite", "copy_path", "-", "file_manager", "terminal"]
```

An icon belongs to one rail and is ignored on the other. A list that names no icon gives that
rail's default.

### Shortcuts

The commands are the rail icons' ids, `back`, `forward`, `parent`, `favorites`, `places`,
`recent_folders`, `recent_files`, `new_folder`, `find`, `favorite`, `copy_path`, `file_manager`,
`terminal`, plus `settings` and `preview` (hides or shows the preview panel for the open dialog):

```
[shortcuts]
places = "SUPER+ALT+P"
find = "F9"
```

A combination is modifiers (`SUPER`, `CTRL`, `ALT`, `SHIFT`) and a key, joined by `+`, and needs
Super or Alt unless the key is F1 to F12. While a dialog is open, ofolder binds each one in
Hyprland to run `ofolder do <command>`, and removes the binding when the dialog closes; a
combination your own Hyprland configuration already binds is skipped. `ofolder do <command>` can
also be run by hand or from your own bindings. If a crash ever leaves a binding behind,
`ofolder unbind-all` removes every one ofolder made.

### Messages

Every message the rails show is also written, one line each with its time, kind and the action
behind it, to `~/.local/state/ofolder/messages.log` (`$XDG_STATE_HOME/ofolder/messages.log`;
`OFOLDER_MESSAGES` overrides it). Only the last 200 lines are kept, and the file is readable by
you alone. The settings window's Messages pane lists the most recent. On screen, an error stays
at least 10 seconds, longer for a long one, and does not go while the pointer rests on it; a click
dismisses it. A passing note from another action never replaces an error you have not read.

## What ofolder keeps, and how to forget it

Everything stays on this machine, in files only you can read (mode `0600`, in folders `0700`):

| What | Where |
| --- | --- |
| Folders and files confirmed in a dialog, with the application that asked | `~/.local/share/ofolder/recents.json` |
| Where each application's dialog was last placed | `~/.local/share/ofolder/dialogs.json` |
| Favorites | `~/.config/ofolder/favorites.json` |
| The last 200 messages the rails showed, with paths | `~/.local/state/ofolder/messages.log` |
| The last check for a new release | `~/.local/state/ofolder/update.json` |

A file that is written by more than one dialog at a time has a `<name>.lock` beside it; the lock
file holds nothing and is safe to leave. Recents older than `recents_days` are dropped, and the
folders listed under `recents_excluded` are never recorded. `ofolder forget`, or the settings
window's "Forget all recents…", empties the recent folders and files and lists what went; favorites
stay. Nothing is sent anywhere.

## Doing it for you

`ofolder doctor` checks all of the above and says what is wrong and how to fix it. `ofolder doctor --report`
prints the same with the versions of ofolder, Hyprland, the portal frontend, Flea and Strata and the plain
on/off settings, and with your home folder written as `~` and your user and host name replaced, so it can be
pasted into a bug report as it is. It names no folder, file or application, and says what it leaves out.

`ofolder repair` does the steps for you without touching anything that is already right: it writes the two
files only if they are missing or point at a program that is gone, puts ofolder first in the chain while
leaving every other line and backend alone, keeps a copy of the file it changed, and restarts the portal
frontend (unless a dialog is open). `ofolder repair --undo` puts the file back as it was, and the guard then
leaves the chain alone until you run `ofolder repair` again, so an undo is not reversed behind your back.

`ofolder portal --check` starts the portal's set-up without claiming its name and says whether it could
serve. Its exit status is 0 when it could, 3 when it could not (the configuration or the backend is broken),
4 when the check itself could not be made (the session bus did not answer), and 1 for anything else, such as
a build too old to know the option.

### The guard

The guard is on by default (`guard = true`; the settings window's General pane has the switch, and turning
it off asks first, saying what you lose). It is `ofolder-guard.sh`, plain shell, with two systemd user units
that run at login and whenever the program's folder or the portal configuration changes. The first dialog
after a fresh start puts it in place; `ofolder repair --watch on|off` does the same by hand.

- If another program has taken ofolder out of the chain, it is put back first and you get one notification.
- If the ofolder program is deleted, will not answer `ofolder version`, or answers it but fails
  `ofolder portal --check` (three times, a few seconds apart, so a bus that was restarting at the wrong moment
  cannot trigger it), its portal file is set aside so file dialogs use the next backend instead of failing
  everywhere. It is put back when a working program returns, and you are told both times.
- A run with nothing changed since the last good one does nothing and does not run the ofolder program
  again (`OFOLDER_GUARD_FORCE=1` makes it look anyway).

What it cannot catch: a build that passes `portal --check` and then fails while serving a dialog, and a
machine with no systemd user session.

## Telling you things, once

- The first dialog ever shows a note saying what the two rails hold. A click closes it, and it never
  returns (`~/.local/state/ofolder/rails-hint-shown` is the mark; delete it to see the note again).
- ofolder reads interfaces that Flea and Strata do not document, so the versions it was tried with are
  recorded in the program (Flea 0.4.0, Strata 1.1.1). When the backend is another version, the first dialog
  with it shows one line saying so, once per version, and `ofolder doctor` says it every time. This means
  "not tried", not "broken"; `ofolder pause` is the way out if the rails misbehave.
