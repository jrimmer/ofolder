# Changelog

## 1.0.1 (2026-11-07)

Fixes the two problems 1.0.0 had in daily use, and quietens the rails.

- **Up, Back, Forward and the clock no longer refuse.** On a desktop where the keyboard follows the
  pointer, a click on a rail used to leave the dialog without the keyboard, and Ofolder refused with
  "the dialog is not the active window". It now takes the keyboard for each group of keys, sends them,
  and puts the pointer back straight afterwards.
- **The rails no longer crash after quick repeated clicks.** The graphics library the rails are built
  on could draw into a window it had already let go of. 1.0.1 carries a small patched copy of it.
- **Quieter.** The notes that only confirmed something (favorite added or removed, folder made, path
  copied, opened in a terminal or file manager) are gone. What is left shows in a bar over the top of
  the dialog instead of beside a rail: failures, the first-dialog welcome (without a coloured edge) and
  the note about a backend version nobody tried.
- **Settings.** Messages now sits above the version, with a button that copies the versions and the
  messages for a bug report, with folder names left out. The version line says what Ofolder sits beside, such as "With Flea v0.4.0".
  For the GTK portal the line gives the portal's own version, since GTK's would be the wrong number.
  "Keep the dialog open after opening the file manager" now says what on and off do, and the note under
  "Keep Ofolder working" lines up with its label.
- **New folder.** The name now starts at the left of its box.
- **Easier to diagnose.** A refused focus check says which window held the keyboard, and a crash of a
  rail says where it happened, both in the journal.

## 1.0.0 (2026-11-06)

The first release.

- **Two rails beside the dialog.** On Omarchy, ofolder docks a slim rail of icons on each side of the
  save and open dialogs of Flea (0.4 and 0.3), the GTK portal and Strata, without changing any of them.
  The left rail goes back, forward and up, and opens favorites, places and recent folders and files.
  The right rail makes a folder, finds one, adds the current folder to your favorites, copies its path,
  opens it in your file manager or a terminal, and opens settings.
- **Preview.** A picture or video selected in an open dialog is shown under it.
- **Favorites, recents and find.** Favorites can be renamed, reordered and removed. Recent folders and
  files can be limited to the application that asked. Find ranks the folders ofolder knows as you type.
- **Private by default.** Everything stays on your machine in files only you can read. Folders that hold
  keys, passwords and browser profiles are never recorded as recent, recents expire after 90 days, and
  `ofolder forget` empties them.
- **A settings window** for the rails, preview, shortcuts, the applications and folders to leave alone,
  starting folders and updates, with a note of what the rails said.
- **It keeps dialogs working.** If ofolder is missing or broken, file dialogs fall back to the next
  backend instead of failing. A small guard puts ofolder back if another program takes it out of the
  portal configuration. `ofolder doctor` says what is wrong, `ofolder repair` puts it right, `ofolder pause`
  turns it off for every dialog, and `ofolder doctor --report` makes a redacted report for a bug report.
- **Updates** are looked for once a week through [praeco](https://github.com/jrimmer/praeco), and
  installed only when you choose; switch the check off in settings.

Needs Hyprland. ofolder was tried with Flea 0.4.0, Flea 0.3.12, xdg-desktop-portal-gtk 1.15 and Strata
1.1.1; another version is said once, not blocked.

## Install

```sh
curl -fsSL https://github.com/jrimmer/ofolder/releases/latest/download/install.sh | bash
```

The script checks each download against its SHA-256, installs under `~/.local` with no root, and then
switches ofolder on (pass `--no-setup` to skip that). To remove it: `install.sh --uninstall`.

Or download `ofolder-linux-x86_64.tar.gz`, check it against `SHA256SUMS`, and extract it into `~/.local`:

    tar -xzf ofolder-linux-x86_64.tar.gz -C ~/.local --strip-components=1

Or build from source with `bin/install`.
