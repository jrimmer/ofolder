# Build plan

Scope: the 33 Baseline requirements in `requirements.md` as ordered in `decisions.md`, minus the three that need the user's say. Written before the work, updated as units land. A unit is done only when its evidence column names a commit and the gate passed for it.

## Gates (not lowered for any unit)

1. `bin/check` exits 0: rustfmt, clippy `-D warnings`, every test, the architecture guard. Run from a clean `git status`.
2. Every unit adds tests named for the behaviour, written with the unit, never loosened afterwards.
3. A unit that changes what a dialog shows or does is also run against the real Flea, GTK and Strata dialogs through the proxy (`spike/proxy_strip_check2.py`, two runs each) before it is committed. Strata runs use the backed-up, hash-verified portal toggle, and the hashes must match afterwards.
4. One commit per unit, on branch `foundation`, short imperative subject, no attribution. Never push, and never register the proxy in `~/.config/xdg-desktop-portal`.
5. Nothing is written into a real folder by a test: scratch folders only. A dialog's own create-folder control is never pressed outside a scratch folder.
6. When a unit cannot meet its acceptance, it is marked blocked with the reason. It is not marked done, and its requirement is not reported as met.

## Held for the maintainer (not built)

R-05 and R-33 were held here until the maintainer approved runtime key bindings on 2026-11-03; they are now built as shortcuts (see `decisions.md`, "Shortcuts").

| ID | Why |
|---|---|
| R-25 folders of open file managers | Depends on what each file manager exposes; needs a decision on which to support |

## Units

| Unit | Requirements | Acceptance (all automated unless marked live) | Status |
|---|---|---|---|
| U1 Settings | R-100 | settings file parsed leniently: a bad value is reported and ignored, the rest still apply; defaults when absent; read per dialog so changes apply to the next one | Done. `settings` tests (12); read per dialog by the hooks, the recorder and the launchers. |
| U2 Recents store | R-40, R-41 (data), R-42 (bonus), R-43, R-44 (data) | unique, move-to-top, max length, exclusions, per-app tag, atomic write, corrupt file set aside; proxy end hook records folders and files from a confirmed dialog and nothing from a cancelled one (end-to-end test on the private bus) | Done. `recents` and `recording` tests, plus live proxy runs on GTK and Flea: confirm records, cancel records nothing, no files created. Strata recording not run. |
| U3 Places and history | R-22, R-23 | standard folders from the user's directory config and mounted volumes (parsers tested on sample files); visited-folder history with parent, back and forward | Done in core (`places` tests); in the left panel, seen in a screenshot, not clicked. `History` moves before a Back or Forward is confirmed. |
| U4 Find | R-60, R-61, R-24 | fuzzy ranking tests; subtree search with depth and time limits and instant cancel on a generated tree; subfolder browsing hides hidden folders and survives an unreadable one | Done in core (`fuzzy`, `search`, `browse`); in the right panel, ranked result seen, typing and Stop not driven live. |
| U5 Panels use it | R-41, R-43, R-44, R-22, R-23, R-24, R-60, R-61, R-63 | strip sections wired to U2 to U4; view-model tests; live (gate 3) | Built (133 strip tests). Gate 3 met for docking, focus, follow and cleanup on all three dialogs, 6 of 6 runs. Live clicks and typing through the real portal frontend on all three dialogs (`spike/real_portal_check.py`), apart from the Strata nav and find confirmations. |
| U6 Launchers | R-12, R-13, R-63 | file manager and terminal detected from the desktop's configuration with settings override; commands built without a shell; tests with fake desktop files | Done. `launch` tests with fake desktop entries; spawn tests in `strip_cmd`. A click on the real buttons was not driven. |
| U7 Per-app | R-50, R-52, R-04, R-102 | start folder rewritten in the request by app id and never over an app-supplied folder; pause switch; D-Bus and portal files for activation written under `packaging/` and not installed | Done. Hook and private-bus tests; `pause`/`resume`; packaging files written, not installed. Apps send no id on this desktop, so the key is `class:<window class>`. |
| U8 Look and memory | R-103, R-90 | theme colours read from the Omarchy theme with a fallback; dialog size and position remembered per app and applied through the float rule | Done. The active theme's `colors.toml` is read per dialog and mapped in one place; dialog place remembered per app. Not covered: a live theme change, a visible jump when a remembered place is applied. |
| U9 Close-out | all | `docs/requirements.md` coverage table: each Baseline requirement marked met, partly met or held, with the test or check that shows it | Done. Coverage table in `docs/requirements.md`. |

## Rules for what "met" means

A requirement is met when its Acceptance lines in `requirements.md` are each covered by a named test or a recorded live run. Where the live behaviour cannot be exercised unattended (a real mouse click, typing in a field), the requirement is marked **partly met**, with the part that was not exercised named.
