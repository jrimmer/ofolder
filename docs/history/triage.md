# Baseline triage

> **Historical.** Written early in the project, before the Rust implementation, and kept for how the design was reached. It is not kept up to date: for what is built and how it was proven, see [`../requirements.md`](../requirements.md) and [`../decisions.md`](../decisions.md).

Each Baseline requirement in `requirements.md`, against what `docs/adapters.md` measured. Status is about feasibility on this machine today, not effort.

| Status | Requirements |
|---|---|
| **Verified in the spike** (all three dialogs) | R-10 show the current folder; R-20 send the dialog to a folder, keeping the filename; R-70 create a folder in the dialog's real folder and enter it |
| **Plain application work** (no dialog access beyond the above) | R-06 long lists; R-11 copy path; R-30, R-31, R-32 favorites; R-41 pick from recents; R-42 forget recents; R-60 fuzzy find; R-100 settings; R-101 local data; R-104 fail visibly |
| **Needs the portal backend (not built)** | R-40 remember folders used: it needs to know a dialog was *confirmed* and with which result, which only the proxy sees |
| **At risk** | R-01 strip lifecycle: the strip must not take focus from the dialog (untested), and two simultaneous dialogs in one resident daemon cannot be told apart, so docking is refused; R-05 keyboard hand-off: moving focus between dialog and strip needs a compositor key binding, and changing Hyprland configuration needs the maintainer's say-so |

## Design points the requirements surfaced

- **Pre-seeding beats driving.** Per-app and per-type starting folders (R-50, R-51, R-52, all Next/Later) need no READ or DRIVE: the portal backend can rewrite `current_folder` and `current_name` in the request before forwarding it. Verified for Flea (the picker honours the folder it is started with). Unverified for the GTK and Strata dialogs.
- **The app id comes with the request.** The portal request carries the calling app's id, so per-app rules do not need to match window classes.
- **Open questions already answered** by decisions: Q1 (portal dialogs only, `docs/adapters.md` scope) and Q2 (show "unknown", never guess, `AGENTS.md` rule 3). Q11: recommend that the first release only moves dialogs and creates folders, and defers R-71 (destructive file actions).
- **Global shortcuts (R-33)** are not available to a Wayland client; they would be compositor key bindings that act on the focused dialog. Treat as Next, behind an owner decision on touching Hyprland configuration.
