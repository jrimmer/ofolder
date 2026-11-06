# Updates

ofolder can tell you when a newer release is out and install it. The work is done by praeco, a
separate static program (https://github.com/jrimmer/praeco). ofolder runs it with fixed argument
lists, never through a shell, and reads what it prints. praeco draws nothing: the window shows
ofolder's own Updates section.

## What happens

- **Checking** is on by default (settings `updates = true`). The portal checks a minute after it
  starts, then once a week. A failed check is not tried again for a day. A check is `praeco check`
  with the release source, the running version and this program's path; its answer is written to
  the state file. A failed check writes nothing, so no pill appears.
- **Showing**: the settings window, General, Updates, shows the last check and, when a newer
  release is offered, a pill "x.y.z available · What's new". The pill opens the list of releases
  newer than the running one, with their notes (up to the `## Install` heading).
- **Installing**: Install runs `praeco install` detached, with `--restart-pid` set to the portal.
  praeco stops the portal (SIGTERM), installs the release, and starts `ofolder status` as its
  restart command. The next dialog starts the new portal through D-Bus activation. The window
  follows praeco's progress file and shows its plain-language steps. When praeco needs a password
  prompt it cannot show, the window offers "Install in a terminal window", which runs praeco
  with `--mode terminal`.
- **Later** hides the pill until the next check. **Skip this version** remembers the version in
  the state file, and only a newer release brings the pill back.
- **Registration with `omarchy update`** is off by default (settings `update_with_system = false`).
  Turning it on asks first. It runs `praeco init --name ofolder ... --hook auto`, after which
  `omarchy update` also runs praeco for ofolder. Turning it off runs `praeco init --remove`.
  The same is available from the command line: `ofolder update register` and
  `ofolder update unregister` (both also record the setting). Nothing registers on its own.
- `ofolder doctor` says whether praeco was found and where, and whether checks are on and whether
  ofolder is registered. A missing praeco and a mismatch between the setting and the registration
  are notes; they do not change the exit status.

## Where praeco is looked for

Never on `PATH`. Unless `OFOLDER_PRAECO` is set, the first executable file of these is used:

1. `<directory of ofolder>/praeco`
2. `<directory of ofolder>/../lib/ofolder/praeco` (the install puts it at `~/.local/lib/ofolder/praeco`)
3. `/usr/lib/ofolder/praeco`

When none is found, the update UI is hidden and nothing is checked.

## Files

| Path | Written by | Holds |
| --- | --- | --- |
| `$XDG_STATE_HOME/ofolder/update.json` (default `~/.local/state/ofolder/update.json`) | the portal after a check; the window for Later and Skip | the last check, the latest release, the release notes, the skipped version, the "later" mark |
| `$XDG_RUNTIME_DIR/ofolder/update-<pid>-<time>.jsonl` | praeco during an install | progress lines; the window reads them and praeco leaves them |
| `$XDG_CONFIG_HOME/ofolder/settings.toml` | the settings window and `ofolder update` | `updates`, `update_with_system` |
| `~/.config/praeco/apps.d/ofolder.json` | praeco, on registration | praeco's own record of the registration |

The state file is private (`0600`) and written under a lock. A damaged state file reads as "no
check yet".

## Environment variables

These are for development and tests. Users need none of them.

| Variable | Effect |
| --- | --- |
| `OFOLDER_UPDATE_SOURCE` | Where releases come from: `forgejo BASE OWNER/REPO` or `github OWNER/REPO`. Unset, the source is the GitHub repository built into `crates/core/src/update.rs` (`DEFAULT_REPO`). A value that does not parse turns checks off, with a message. |
| `OFOLDER_PRAECO` | The praeco to run. When set, nothing else is searched, and a path that is not an executable file turns the update UI off. |
| `OFOLDER_PRETEND_VERSION` | The version the check reports as running, for example `0.0.1`. Only the update code reads it. |
| `OFOLDER_UPDATE_CHECK_DELAY` | Seconds before the first check (default 60). Setting it also forces that first check, even when one was made this week. |

Releases are signed. praeco checks each download against the signature made by the release workflow, and refuses a release without a valid one (see `docs/releasing.md`).

## Testing without a release

Point praeco at the fake script and the portal at a short delay:

```sh
mkdir -p /tmp/ofolder-fake && ln -s "$PWD/tests/support/fake-praeco" /tmp/ofolder-fake/praeco
echo '{"ok":true,"latest":"9.9.9","update_available":true,"releases":[],"install":{"kind":"tarball","supported":true}}' \
  > /tmp/ofolder-fake/check.json
OFOLDER_PRAECO=/tmp/ofolder-fake/praeco OFOLDER_UPDATE_CHECK_DELAY=0 ofolder portal --backend gtk
```

`tests/support/fake-praeco` answers `check` from `check.json` and `install` from
`install-dialog.jsonl` (or `install-terminal.jsonl`) in the folder it is run from, and records every
invocation in `calls.log`. The file names are listed at the top of the script. A link is used
rather than a copy in the tests, because a freshly written script can fail with "text file busy"
under parallel tests.

The app tests (`crates/app/src/update.rs`) drive the same code through this fake in temporary
folders, and never read the maintainer's configuration or run the real praeco.

## Quitting for an update

praeco restarts the portal with SIGTERM and waits for it to exit. The portal catches SIGTERM,
drops its bus name and exits with status 0. It does not stop the strips of dialogs that are open:
each strip ends when its dialog closes, as it does when the portal crashes. Stopping one in the
middle of a dialog would leave its key bindings behind. So an update that happens while a dialog
is open ends that dialog's request, and the next dialog uses the new portal.

## Interlock with the guard and `portal --check`

An update replaces the program the portal starts. A release that cannot start would stop every file
dialog, so there are three layers, each catching what the one before it cannot:

1. **Before publishing**, the release rehearsal in `docs/releasing.md` runs `portal --check` on the
   packaged binary. A release is only published after it passes.
2. **At install**, `bin/install` runs `portal --check` before it swaps a build. praeco's own install
   does not; it replaces a tarball install without asking ofolder.
3. **After any swap**, the guard (`ofolder repair --watch on`, the default) wakes when the program's
   folder changes and runs `portal --check` on what is now installed. When that fails with status 3
   twice more, a few seconds apart, the guard sets ofolder's portal file aside and tells you: file
   dialogs then use the next backend (Flea, GTK) instead of failing, and ofolder returns on its own
   when a working build is put back (`bin/install --rollback` from a checkout restores the previous
   one). A build that fails to start at all, or is deleted, is handled the same way.

What is still not covered: a build that passes `portal --check` and fails later, while serving a
dialog (recorded as an expected failure in the end-to-end suite); the guard turned off in the
settings; and a machine with no systemd user session, where nothing watches.

## Not done yet

- The pill is shown only in the settings window. The strips do not show one; that needs a place
  in the rails.
- The praeco version that `bin/install` places is the one `scripts/get-praeco` pins.
- The source repository is private, so checks fail until it has public releases or praeco is given
  a token (its default variable is `PRAECO_TOKEN`). ofolder passes no token.

See `docs/releasing.md` for the release steps, the asset names praeco expects, and the release
rehearsal.
