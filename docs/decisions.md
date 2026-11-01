# Owner decisions

Recorded from the requirements review of 2026-11-01. `requirements.md` carries each changed priority with a `Decision` line. Where this file and the older notes disagree, this file wins.

## Result

| | Documented | After review |
|---|---|---|
| Baseline | 17 | **33** |
| Next | 14 | 4 |
| Later | 10 | 2 |
| Dropped | 0 | 2 |

23 priorities changed, and R-71 was then set to Next. 17 requirements were promoted into Baseline, 1 moved down (R-42 to Next), 2 moved up from Later to Next (R-51, R-71), and 2 were dropped.

## Dropped

- **R-10, showing the dialog's current folder.** The dialog already shows it. The strip still reads the folder internally, because copy path (R-11), add favorite (R-31), recents (R-40) and new folder (R-70) all depend on it. What goes is the on-screen display. A small "folder unknown" notice is still worth showing, so a disabled action is explained.
- **R-21, typing a path to go there.** Each dialog has its own location field and accepts a pasted path.

## Open questions, as answered

| | Answer |
|---|---|
| 1. Which dialogs | Portal dialogs only for the first release |
| 2. Folder cannot be read | Show "unknown" and switch off what needs it; never guess |
| 3. When to record a recent folder | When a dialog is confirmed with a result, not on browsing (needs the portal backend) |
| 4. Recents global or per app | Both: everyone's by default, with a per-app view |
| 5. Keep the typed file name on a jump | Always |
| 6. Delay before the strip appears | Under one second, and as quickly as possible |
| 7. Default file manager and terminal | Detect from the desktop's configuration, with settings to override |
| 8. Override an app's own start folder | No: only set one when the app supplied none |
| 9. Strip taking focus | Never automatically; only when the user presses its shortcut |
| 10. Data format | Plain TOML for settings, JSON for lists, under `~/.config/ofolder`, documented and stable |
| 11. Destructive file actions (R-71) | First release only moves dialogs and creates folders; defer R-71 |
| 12. Monitors and tiled dialogs | Flip at a monitor edge, follow across workspaces, float a tiled dialog with a rule before it opens |

## Conflicts, now resolved

1. **R-71 versus answer 11.** Resolved: R-71 is **Next**. The first release only moves dialogs and creates folders.
2. **R-02 and the layout note.** Resolved: **one panel on each side of the dialog**. See the next section.
3. **Answer 4 says "per-app view (R-43, Next)"**, and R-43 was then promoted to Baseline. Consistent, but the answer's text is stale.

## Layout: a panel on each side

Decided by the maintainer on 2026-11-01. The left panel holds the click-to-select icons and navigation. The right panel holds the more interactive components.

| Panel | Holds |
|---|---|
| **Left: go somewhere** | favorites (R-30), standard locations and drives (R-22), recent folders (R-41, R-43), back, forward and parent (R-23), folders of open file managers (R-25) |
| **Right: do something here** | the "folder unknown" notice, copy path (R-11), add or remove favorite (R-31), new folder (R-70), find (R-60, R-61), open in file manager or terminal (R-12, R-13), settings and pause (R-102) |

Placement rules, in order: the left panel sits left of the dialog and the right panel sits right of it. If one side has no room, both panels pack onto the other side, in the same order. If neither side has room for both, they overlap the dialog's edge. R-02 as documented ("choose which side") becomes: the roles are fixed, and a setting may swap the two sides later.

## Feasibility flags on the promoted items

| ID | Concern |
|---|---|
| R-33 shortcut per favorite | A Wayland client cannot capture global shortcuts. It needs compositor key bindings, which means changing Hyprland configuration. |
| R-05 keyboard hand-off | Same constraint, and it conflicts with answer 9 unless the shortcut is the only way focus moves. |
| R-25 folders of open file managers | Depends on what each file manager exposes. Likely partial. |
| R-52 document's own folder | Most apps do not tell the dialog the document's folder. It will only work for those that do. |
| R-04 start with the session | The portal backend is started by D-Bus on demand, so nothing needs autostarting once it is registered. |
| R-90 dialog size and position | Feasible with the compositor, but overlaps with the float rule used for Strata. |
| R-40, R-43, R-44 | Need the portal backend to see what a dialog returned; the proxy does. |

## Suggested order for the 34

Ordered by dependency, not by importance. All 34 remain Baseline.

1. **Two panels that work**: R-01, R-02, R-06, R-20, R-30, R-31, R-32, R-70, R-11, R-100, R-101, R-104, R-60. Mostly built or proven.
2. **Memory**: R-40, R-41, R-43, R-44, plus the end hook in the proxy.
3. **Going places**: R-22, R-23, R-24, R-61, R-63, R-12, R-13.
4. **Per-app behaviour**: R-50, R-52, R-03, R-04, R-102, R-103, R-90.
5. **Needs your say first**: R-05, R-33, R-25. R-71 is now Next.
