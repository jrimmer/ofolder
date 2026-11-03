# ofolder: picker adapter findings (foundation)

ofolder docks a strip of chrome (favorites, new folder, go-to-folder) beside a save/open dialog and drives it.
There is no supported API for that, so each dialog needs an **adapter** that says honestly what it can do.
Everything below was measured on host (Hyprland 0.56.2, Flea 0.3.12, xdg-desktop-portal-gtk 1.15.3, Strata 1.1.1) on 2026-11-01.
Evidence: `spike/runs/` (logs, screenshots). Code: `spike/`.

## Scope: portal dialogs only (decided 2026-11-01)

ofolder can only decorate dialogs that applications request through the desktop portal, because that is the path it plugs into. An application that draws its own dialog inside its window never passes through ofolder, and no strip appears. Many ordinary GTK and Qt applications do this unless they are set up to use the portal. This is a known limit, accepted as the place to start; per-application dialogs are out of scope for now. First follow-up: find out which of the everyday applications on this machine actually show the portal picker when saving, since those are the ones covered.

## What an adapter must provide

| Capability | Meaning |
|---|---|
| `launch` | start the dialog and learn which window is ours |
| `window` | find the dialog's window reliably (never by window address: Hyprland reuses them) |
| `ensure_floating` | make the window float and a sane size before docking |
| `read_folder` | the folder the dialog is showing, or **None**. Never guess |
| `goto(path)` | move the dialog to a folder; the caller verifies arrival with `read_folder` |
| `close` | cancel the dialog without keystrokes where possible |

If `read_folder` is None the strip disables "New folder" instead of guessing. After `goto`, verify arrival and fall back (respawn, or a different method).

## The three backends

| | Flea | GTK portal (`xdg-desktop-portal-gtk`) | Strata (`strata --portal`) |
|---|---|---|---|
| Process model | one child `flea --pick` per request | one resident daemon hosts all dialogs | one resident backend, D-Bus activated, idles out after ~2 min |
| Launch | `flea --pick <reply>` with request JSON in env `FLEA_PICKER` (read from `/usr/lib/flea/flea-portal`; undocumented) | D-Bus `org.freedesktop.impl.portal.desktop.gtk` `SaveFile` | D-Bus `org.freedesktop.impl.portal.desktop.strata` `SaveFile` (needs `strata --install-portal`) |
| Window | class `com.thisisgm.flea.picker`, floats, match by **pid** | class `xdg-desktop-portal-gtk`, floats, match by title | class `io.github.lgse.Strata.FileChooser`, opens **tiled and squashed**: adapter must float and resize it |
| `read_folder` | the picker's own `snapshot` IPC (`path`, ~20 ms), checked against the directory fd the `flea --backend` helper holds (`/proc/<pid>/fd`, ~75 ms), which is also the fallback; see Selection | AT-SPI: path bar toggle buttons, the CHECKED one is current, rebuild path up to `File System Root`, accept only if `isdir` (~41 ms) | AT-SPI: join the breadcrumb labels beside the `Copy path` button (`~`, `/`, `Work` becomes `~/Work`) |
| `goto` | `wtype` Ctrl+L, path, Return into the verified-active picker (~75 ms) | **no keys**: set the Name entry text to `path/` via AT-SPI EditableText, then fire its `activate` action **twice**, 0.4 s apart (the first is swallowed after a move; 24/24 with two) | places buttons and `Parent folder` have `click` actions (no keys); arbitrary path via Ctrl+L keys (7/7, 67 to 189 ms) |
| `close` | Escape (needs the picker active) | `Cancel` button action | `Cancel file selection` button action |
| Native new-folder | none (the reason for the strip) | `Create Folder` button | `New Folder` button |
| Does fd reading work? | yes | no (no dir fds; inotify watches are autocomplete noise, **not** a signal) | no (no children, no dir fds) |

## Reference implementation

`spike/strip_multi.py`: one backend-agnostic strip plus the three adapters above (`SPIKE_ADAPTER=flea|gtk|strata`, `SPIKE_AUTO=1` for the unattended run). 2026-11-01 results, evidence in `spike/runs/multi-*/`: Flea 7/7, GTK 8/8, Strata 7/7. Each run checks: the strip docks flush beside the dialog and floats; the folder reads correctly; a strip button moves the dialog; "New folder" is created in the dialog's real folder and entered; the strip follows when the dialog is moved; the dialog closes and the strip exits. The strip's own buttons were fired from code, not by a mouse click.

## Rust implementation status (2026-11-01)

- `crates/core`: the `Adapter` trait, `goto_verified` (a move counts only when the dialog is seen to arrive), stable folder reading, the pure readers, placement geometry, path guard. 58 tests.
- `crates/adapters`: the Flea adapter, `hyprctl`, `wtype` and `/proc` access, behind the `Compositor` and `Typist` traits. 20 tests with fakes, covering: never type unless the picker is the verified-active window, stop when focus leaves mid-move, refuse paths that would act as keys, match the window by pid.
- `crates/portal`: the file-chooser proxy (`ofolder portal --backend <id>`). 12 tests, including 6 end-to-end on a private D-Bus with a fake real backend: forwarding, rewriting the start folder before the dialog sees it, the end hook and guard, cancel reaching the real dialog, an unreachable backend answered as "other", request paths reusable.
- Live proof: `ofolder live-check` runs the Flea adapter against the real Flea. 10 of 11 runs passed. The one failure was the first run after idle: both moves were issued and the picker went nowhere, cause not found, which is why moves are verified. The proxy was also run on the live session bus against the real GTK portal dialog: the dialog opened through it, accessibility read its folder, Cancel came back through the proxy as response 1. No portal configuration was changed for either.
- Not yet in Rust: the GTK and Strata adapters (accessibility), the strip wired to a dialog, the hook that opens the strip beside the dialog, and the Hyprland window-rule handling.

## Selection (spike, 2026-11-03)

The preview panel under the dialog needs the one item selected in the dialog's file list. `Adapter::selection` returns it as a path that exists, or `None`. Nothing is typed or pressed to read it. A name read from the dialog is turned into a path only when it is one plain file name inside the folder the dialog shows (read before and after, and the two must agree) and that path exists; several selected items, an empty row or a name that cannot be tied to one path give `None`.

| | Flea | GTK portal | Strata |
|---|---|---|---|
| Can it be read? | **yes** | **yes** (the selection; a cursor move by key was not seen to change it) | **yes** in Strata's own window; the chooser itself not run (see below) |
| How | The picker's own Quickshell IPC: `qs ipc --pid <picker pid> call fleapicker snapshot`, a read-only seam in `PickerWindow.qml` (target `fleapicker`). One call returns the folder (`path`), the cursor row's name (`cursorName`) and the marks at one instant. A single mark wins; several marks are no single selection; otherwise the cursor row. | AT-SPI: the table named `Files`, four column headers then four cells per row; the first cell of a selected row (state SELECTED) holds the icon and the name. | AT-SPI: each Miller column is a `list` named after its folder; its `list item` with state SELECTED, in the column named after the folder shown. Two columns of that name are ambiguous and give `None`. |
| Cost per read | about 20 ms (one `qs` process per call) | about 140 to 160 ms through the Rust `atspi` view (one D-Bus round trip per property, about 45 cells); 0.7 ms with the Table interface's `get_selected_rows` in Python | about 40 to 50 ms (Python walk) |
| Update after a change | the next read (20 ms after a key) | not measured: Down keys sent to the verified-active dialog did not move GTK's selection | the next read (about 110 ms including the read) |

Notes and hazards:

- **Flea's folder from the same IPC (adopted 2026-11-03).** In a scratch folder under `/tmp` the `--backend` helpers once held **no** directory descriptor at all, so the fd reader answered unknown while `snapshot`'s `path` was right. `read_folder` now asks `snapshot` first and uses the descriptors only when it gives no folder; if both answer and disagree the folder is unknown. One `snapshot` serves the folder and the selection within 120 ms, so a poll makes one call. Measured 2026-11-03: about 20 ms per call in a 6-entry scratch folder and in `/usr/bin` (3,900 entries: the picker pages its `rows`, 84 sent, 12 KB), so the cost does not grow with the folder. `ofolder live-check flea` passed through `/tmp` scratch folders, a non-ASCII name and home.
- **Flea has no accessibility tree.** The picker registers on the AT-SPI bus with no children. `org.a11y.Status.IsEnabled` is false on this desktop, and turning it on would change the system; the picker's rows declare no `Accessible` properties either. Not pursued.
- **Flea's Recent view** reports a `path` that is not a folder; its rows give `None`.
- **Single-file Open dialogs** in Flea keep the marks empty (`v` and Space did not mark); the cursor row is the selection, as Flea's own "a lone selection follows a plain move" says.
- **GTK.** The first row is selected when the dialog opens. Only the visible rows are in the tree, which is enough for the selected one in practice. The read is slow through the Rust view; poll it at most twice a second, and only while the preview panel is open. A faster read would use the Table interface (`get_selected_rows`, `get_accessible_at`). As before, never call `Selection.select_child` or `Component.grab_focus` on this dialog.
- **Strata's chooser was not run.** Enabling its portal backend (`strata --install-portal`) rewrites the portal configuration and restarts the portal frontend, which would have clobbered the user's live `FileChooser=ofolder;flea;gtk` line and killed a dialog the maintainer had open. The reading was proven on Strata's main window on the same folder (its documentation says the chooser uses the same views); it must be confirmed on the chooser before the preview relies on it there.
- **What the preview does without a selection:** shows the folder the dialog is in (its summary), never a guess.

Evidence: `spike/runs/selection/` (notes and probe output). Probe: `cargo run -p ofolder-adapters --example selection_probe -- flea <picker pid>` or `-- gtk "<dialog title>"`, read-only.

## Hazards (all observed)

- **Flea's picker once ignored two moves issued right after launch** (1 of 11 live runs, the first after idle; settle delays did not matter in later runs). Always verify arrival and be ready to retry or respawn.

- **GTK clears the Name entry when a move lands**, and our `goto` uses that entry as a path field, so the app's suggested filename was lost. The adapter now saves it before and restores it after (check: "app's filename survived every move").
- **Strata's dialog opens tiled**, which would reflow your layout. A named window rule (float, size, centred) is installed *before* the request, so it never tiles; the rule is disabled on exit. Strata's own portal had to be enabled to test it; it was removed afterwards with `strata --uninstall-portal` and the config verified byte-identical to a backup.

- **`wtype` types into whatever has focus.** Only send keys after verifying the target is the active window, and re-verify between steps. Typing while the human types interleaves; one typed path arrived garbled (unreproduced in 40 retries).
- **A bad typed path wedges Flea's path bar** (error state): in-place moves stop working. Verify arrival, then respawn.
- **Do not call `Atspi.Selection.select_child` or `Atspi.Component.grab_focus` on the GTK dialog.** They raised `ATK_IS_SELECTION` assertions, and after my probes the GTK portal daemon crashed (SIGSEGV 13:00:51; a cleanup press of Cancel on the damaged dialog is the likeliest trigger, not proven; stack unsymbolized). It restarts on demand and no data is involved. The set-text, activate and Cancel subset ran 100+ times without a crash.
- **AT-SPI objects vanish mid-move** (the path bar is rebuilt). Treat any accessibility error as "no reading yet" and retry.
- Symlinks report their resolved path (Flea); the "new folder" lands in the real folder, which is correct.
- A running Flea or GTK portal can be older than its package (`(deleted)` binary); the fd and AT-SPI behaviour may differ after a restart.

## Fragility and the self-check

Flea's `--pick`/`FLEA_PICKER` contract and the fd trick are undocumented and may change in any `flea --update`. Plan: run a self-check per adapter at startup and after any package version change (spawn a probe dialog, read, move, read, close), and disable an adapter that fails. Separately: draft a docs PR for the `--pick` contract and an issue asking for a supported current-folder signal at github.com/thisisgm/flea. Nothing is sent without Jason's yes, and the text carries no AI attribution.

## Not yet verified

Real mouse clicks on the strip (buttons were fired from code); mouse moves inside the dialogs; sidebar Recent/Trash; multiple monitors; two dialogs at once (resident daemons make matching ambiguous: refuse to dock); Strata's own window-centering rule when called through the frontend (direct calls opened it tiled); the new-folder flow inside GTK/Strata; whether Flea stays the effective default while Strata's portal is enabled (Strata wrote a `portals.conf` that the existing `hyprland-portals.conf` should shadow; inferred from the spec, not tested).
