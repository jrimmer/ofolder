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
