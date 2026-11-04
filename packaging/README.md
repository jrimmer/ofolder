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
and any key it does not know. A value of the wrong type is reported and ignored; the rest
still apply.

| Key | Meaning | Default |
|---|---|---|
| `keep_dialog_after_file_manager` | Keep the dialog open after "open in file manager" | `true` |
| `recents_enabled` | Remember recent folders and files | `true` |
| `recents_max` | How many recent items are kept, 1 to 500 | `20` |
| `recents_excluded` | Folders, and everything below them, never recorded as recent | none |
| `excluded_apps` | Application ids whose dialogs are left alone | none |
| `file_manager`, `terminal` | Commands to use instead of the desktop's own; the folder is added at the end | detected |
| `start_folders` | A table of application id to the folder its dialogs start in, when the app gives none | none |
| `paused` | Set by `ofolder pause`: every dialog passes through untouched | `false` |
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

## Doing it for you

`ofolder doctor` checks all of the above and says what is wrong and how to fix it. `ofolder repair`
does the steps for you without touching anything that is already right: it writes the two files only
if they are missing or point at a program that is gone, puts ofolder first in the chain while leaving
every other line and backend alone, keeps a copy of the file it changed, and restarts the portal frontend
(unless a dialog is open). `ofolder repair --undo` puts the file back as it was.

`ofolder repair --watch on` also installs a small guard (`ofolder-guard.sh`, plain shell) with two systemd
user units, which run at login and whenever the program or the portal configuration changes:

- if another program has taken ofolder out of the chain, it is put back first and you get one notification;
- if the ofolder program is deleted, or is there but will not answer `ofolder version` (a bad build), its portal file is set aside so file dialogs use the next backend
  instead of failing everywhere, and put back when the program returns.

`ofolder repair --watch off` removes it. The guard is optional and starts only because you ran that command.

