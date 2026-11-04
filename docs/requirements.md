# ofolder requirements

ofolder docks a side panel (the "strip") beside a file Open or Save dialog and adds navigation and convenience controls that the dialog itself lacks. Its purpose is to get a person to the right folder in a few keystrokes or clicks, and to make repeated file chores around the dialog quicker, in any application.

## How to read this

### Priorities

- **Baseline**: needed for a first release to deliver the core promise of dialog chrome. A short list on purpose.
- **Next**: high value soon after the first release.
- **Later**: nice to have; revisit once the rest is solid.

### Depends-on tags

Tags describe what the requirement needs from the tool. They exist so builders can triage feasibility; they do not change the priority.

- **READ**: needs the dialog's current folder or other dialog state.
- **DRIVE**: needs to move or operate the dialog (send it to a folder, press its controls).
- **STORE**: needs the tool's own saved data (settings, lists of folders).
- **FS**: needs the tool to perform filesystem operations itself.
- **OS**: needs desktop or system integration (window events, clipboard, global shortcuts, autostart, launching other programs).
- **NONE**: needs none of the above.

### Conventions

- "Dialog" means a file Open or Save chooser belonging to any application.
- "Strip" means the docked panel.
- "Current folder" means the folder the dialog is showing at that moment.
- Where a requirement says "app", it means the application that owns the dialog, identified by its window class.

## 1. Strip lifecycle and docking

### R-01 Strip appears with the dialog
- **User need**: When I open a file dialog, I want the extra controls to be there without any action on my part.
- **Behavior**:
  - The strip appears next to a dialog soon after the dialog appears and disappears when the dialog closes.
  - The strip follows the dialog when the dialog is moved or resized, and stays on the same workspace and monitor.
  - Multiple dialogs at once each get their own strip.
  - The strip never takes keyboard focus away from the dialog unless the user asks for it.
- **Acceptance**:
  - Opening a dialog from three different applications shows a strip for each within one second.
  - Dragging the dialog moves the strip with it with no visible detachment at rest.
  - Closing the dialog by any means removes the strip; no orphan strips remain after 100 open/close cycles.
  - Typing the file name immediately after the dialog opens still goes to the dialog.
- **Priority**: Baseline
- **Depends on**: OS

### R-02 Choose which side the strip docks on
- **User need**: I want the strip on the side that does not cover other things I need.
- **Behavior**:
  - A setting selects left or right; the strip falls back to the other side when there is no room on the chosen side.
  - Vertical alignment (top or centered) is selectable.
- **Acceptance**:
  - Changing the setting affects the next dialog without restarting the tool.
  - A dialog at the screen edge still gets a fully visible strip.
- **Priority**: Baseline
- **Decision**: moved from Next to Baseline in the user's review of 2026-11-01. Owner note: the strip on the left holds the click-to-select icons and navigation; the more interactive components go on the right. Decided: one panel on each side of the dialog (see docs/decisions.md).
- **Depends on**: OS, STORE

### R-03 Per-app exclusion
- **User need**: If the strip misbehaves in one application, I want to turn it off there only.
- **Behavior**:
  - The user can list app window classes for which no strip is shown.
  - The list can be edited from the strip itself ("don't show for this app") and from settings.
- **Acceptance**:
  - After adding an app to the list, its next dialog shows no strip while other apps still do.
  - Removing the entry restores the strip.
- **Priority**: Next
- **Depends on**: OS, STORE

### R-04 Start with the session
- **User need**: I do not want to remember to launch the tool each time I log in.
- **Behavior**:
  - A setting enables starting the tool automatically at login.
  - The tool runs in the background with no window of its own until a dialog appears.
- **Acceptance**:
  - With the setting on, after a fresh login the strip appears on the first dialog with without starting it by hand.
  - With it off, nothing is started automatically.
- **Priority**: Baseline
- **Decision**: moved from Next to Baseline in the user's review of 2026-11-01.
- **Depends on**: OS, STORE

### R-05 Keyboard-only operation of the strip
- **User need**: I want to use the strip without reaching for the mouse in the middle of a keyboard workflow.
- **Behavior**:
  - A single shortcut moves focus into the strip and another returns it to the dialog.
  - Every strip control can be reached and activated by keyboard.
- **Acceptance**:
  - A person can go from an opened dialog to a chosen favorite folder using only the keyboard.
  - Focus always visibly belongs to either the dialog or the strip.
- **Priority**: Baseline
- **Depends on**: OS

### R-06 Strip adapts to large lists
- **User need**: Long lists must remain usable.
- **Behavior**:
  - Lists longer than the strip's height scroll, or can be filtered by typing.
  - Long folder paths are shortened sensibly while the full path stays available on hover or by command.
- **Acceptance**:
  - A list of 200 entries is fully reachable.
  - A 300-character path does not widen or break the strip.
- **Priority**: Baseline
- **Depends on**: NONE

## 2. Current-folder awareness

### R-10 Show the dialog's current folder
- **User need**: I want to see at a glance where the dialog is pointing.
- **Behavior**:
  - The strip displays the full path of the current folder, and updates when the dialog navigates elsewhere.
  - The display is shortened with the home directory abbreviated, and the full path is available on demand.
  - If the folder cannot be determined, the strip says so rather than showing a stale or guessed value.
- **Acceptance**:
  - Navigate the dialog through five folders; the strip's path matches each within one second.
  - When the folder is unknown, no old path is shown as if it were current.
- **Priority**: Dropped
- **Decision**: dropped in the user's review of 2026-11-01. Owner note: the dialog itself already shows the folder, so the on-screen display is dropped. The strip still reads the folder internally, because R-11, R-31, R-40 and R-70 depend on it.
- **Depends on**: READ

### R-11 Copy the current folder's path
- **User need**: I often need the folder path as text for a terminal, a message or a config file.
- **Behavior**:
  - One action places the current folder's absolute path on the clipboard.
  - A confirmation is shown briefly.
- **Acceptance**:
  - After the action, pasting yields exactly the path displayed, with correct handling of spaces and non-ASCII characters.
- **Priority**: Baseline
- **Depends on**: READ, OS

### R-12 Open the current folder in the file manager
- **User need**: Sometimes I need to do more with the folder than the dialog allows.
- **Behavior**:
  - One action opens the current folder in the user's chosen file manager.
  - A setting chooses whether the dialog stays open or closes afterwards (default: stays open).
  - The file manager is a setting.
- **Acceptance**:
  - The file manager opens showing exactly the dialog's current folder.
  - With "stay" selected the dialog is still present and usable afterwards.
- **Priority**: Baseline
- **Decision**: moved from Next to Baseline in the user's review of 2026-11-01.
- **Depends on**: READ, OS, STORE

### R-13 Open the current folder in a terminal
- **User need**: I want a shell in the folder where I am about to save or open.
- **Behavior**:
  - One action launches the chosen terminal with its working directory set to the current folder.
- **Acceptance**:
  - The terminal's working directory equals the dialog's current folder.
- **Priority**: Baseline
- **Decision**: moved from Next to Baseline in the user's review of 2026-11-01.
- **Depends on**: READ, OS, STORE

## 3. Jumping to folders

### R-20 Send the dialog to a chosen folder
- **User need**: I want to jump the dialog to a folder in one action instead of clicking down a tree.
- **Behavior**:
  - Choosing a folder anywhere in the strip (favorites, recents, search, location shortcuts) sends the dialog there.
  - The dialog's file name field content is preserved in a Save dialog.
  - If the folder no longer exists or is not reachable, the user sees a clear message and the dialog is left where it was.
- **Acceptance**:
  - From a dialog showing folder A, choosing folder B makes the dialog show B, and the strip's current-folder display agrees.
  - In a Save dialog, text typed into the name field before the jump is still there after.
  - Choosing a deleted folder leaves the dialog on A and shows an error.
- **Priority**: Baseline
- **Depends on**: DRIVE, READ

### R-21 Type a path to go there
- **User need**: When I know or have a path in hand, I want to paste it and go.
- **Behavior**:
  - A text entry in the strip accepts absolute paths, paths beginning with the home shortcut, and file URLs.
  - Completion suggests existing folders while typing.
  - A path pointing to a file sends the dialog to the containing folder.
- **Acceptance**:
  - Pasting `~/Documents` or the equivalent absolute path moves the dialog there.
  - A nonexistent path gives an error and no navigation.
- **Priority**: Dropped
- **Decision**: dropped in the user's review of 2026-11-01. Owner note: pasting a path into the dialog's own location field already does this.
- **Depends on**: DRIVE, FS

### R-22 Standard locations shortcuts
- **User need**: Home, desktop, downloads, documents and mounted drives should be one action away.
- **Behavior**:
  - A list of standard user directories (as defined by the desktop's user-directory configuration) and mounted volumes is shown.
  - Removable volumes appear and disappear as they are connected.
- **Acceptance**:
  - Each shortcut takes the dialog to the matching directory.
  - Plugging in a drive adds it within a few seconds.
- **Priority**: Baseline
- **Decision**: moved from Next to Baseline in the user's review of 2026-11-01.
- **Depends on**: DRIVE, OS

### R-23 Go to the parent folder and back/forward through visited folders
- **User need**: I want to step up one level or retrace my steps quickly.
- **Behavior**:
  - Actions exist for parent folder, previous folder and next folder in the history of folders visited via the strip and observed in the dialog.
- **Acceptance**:
  - After visiting A, B, C, the "previous" action goes to B then A, and "next" goes back to B then C.
  - Parent from `/a/b` goes to `/a`.
- **Priority**: Baseline
- **Decision**: moved from Later to Baseline in the user's review of 2026-11-01.
- **Depends on**: DRIVE, READ, STORE

### R-24 Browse into subfolders from the strip
- **User need**: I want to descend several levels without opening each in the dialog.
- **Behavior**:
  - Any folder in the strip can be expanded to show its subfolders, to arbitrary depth, and any level can be chosen as the destination.
  - Hidden folders are excluded unless the user enables showing them.
  - Slow or unreadable directories do not freeze the strip.
- **Acceptance**:
  - A destination four levels deep is reachable and selectable from the strip alone.
  - Expanding a directory on an unresponsive network mount does not block other strip controls.
- **Priority**: Baseline
- **Decision**: moved from Later to Baseline in the user's review of 2026-11-01.
- **Depends on**: FS, DRIVE

### R-25 Jump to the folder of an open file-manager window
- **User need**: If I already have the folder I want open in a file manager, I want to reuse it without retyping.
- **Behavior**:
  - The strip lists folders currently shown by supported file managers and lets me choose one.
  - Entries disappear when the window closes.
- **Acceptance**:
  - With two file-manager windows open on different folders, both appear and each sends the dialog to its folder.
- **Priority**: Baseline
- **Decision**: moved from Later to Baseline in the user's review of 2026-11-01.
- **Depends on**: OS, DRIVE

## 4. Favorite folders

### R-30 Maintain a list of favorite folders
- **User need**: I want a personal list of folders I use constantly, available in every dialog.
- **Behavior**:
  - Favorites persist across sessions and are shown in the strip in the user's chosen order.
  - A favorite has a path and an optional display name.
  - Favorites whose folder is missing remain listed but are visibly marked; choosing one reports the problem.
- **Acceptance**:
  - Restarting the tool or logging out preserves the list and order.
  - A favorite whose folder was renamed is shown as unavailable and does not navigate.
- **Priority**: Baseline
- **Depends on**: STORE

### R-31 Add the current folder as a favorite from the strip
- **User need**: When I am in a folder I will want again, I want to save it right then.
- **Behavior**:
  - One action adds the dialog's current folder to favorites; adding a duplicate is detected and not repeated.
  - A matching action removes the current folder if it is already a favorite.
- **Acceptance**:
  - Using the action in folder X makes X appear in the favorites list and in later dialogs.
  - Using it twice leaves one entry.
- **Priority**: Baseline
- **Depends on**: READ, STORE

### R-32 Edit, reorder and remove favorites
- **User need**: My list changes and must be easy to curate.
- **Behavior**:
  - Entries can be renamed, reordered (drag or move commands), removed, and separated into groups with divider lines.
  - Entries can be added by typed path as well as by pointing at a folder.
  - Editing is possible from a settings view without a dialog being open.
- **Acceptance**:
  - Each operation is reflected in the next dialog.
  - Adding a favorite by typing a path that does not exist is rejected with a message.
- **Priority**: Baseline
- **Depends on**: STORE

### R-33 Keyboard shortcut per favorite
- **User need**: My top few folders should be reachable with one key combination.
- **Behavior**:
  - A favorite can be assigned a key combination that sends the active dialog to it.
  - Conflicts with another assignment are detected and reported.
  - The shortcut only acts while a dialog is active (it does not capture the key elsewhere).
- **Acceptance**:
  - Pressing the combination in a dialog navigates it to the favorite.
  - Pressing it in a non-dialog window passes through to that window.
  - Assigning the same combination twice is refused with a message.
- **Priority**: Baseline
- **Decision**: moved from Next to Baseline in the user's review of 2026-11-01.
- **Depends on**: STORE, DRIVE, OS

### R-34 Named sets of favorites
- **User need**: I work on different projects and want a different list for each.
- **Behavior**:
  - The user can create, rename, delete and switch named sets; exactly one is active.
  - The active set is visible in the strip and switchable from it.
  - Recent folders may be shared across sets (see Open questions).
- **Acceptance**:
  - Switching sets changes the favorites shown in the next dialog and in any open strip.
  - Deleting a set asks for confirmation.
- **Priority**: Later
- **Depends on**: STORE

## 5. Recent folders

### R-40 Remember folders used in dialogs
- **User need**: The folder I used yesterday is probably the one I want today.
- **Behavior**:
  - When a dialog is confirmed (a file is opened or saved) or closed after visiting a folder, that folder is recorded as recently used.
  - Entries are unique; reuse moves an entry to the top.
  - Only existing directories are recorded; the list has a maximum length the user can set with oldest dropped first.
- **Acceptance**:
  - After saving a file into folder X, X is first in the list in the next dialog.
  - After using X twice, it appears once.
  - With a maximum of 10, the 11th distinct folder pushes out the oldest.
- **Priority**: Baseline
- **Depends on**: READ, STORE

### R-41 Show and choose from recent folders
- **User need**: I want to pick from recent folders with one action.
- **Behavior**:
  - Recent folders are listed in the strip, newest first by default, with an option for alphabetical.
  - Choosing one sends the dialog there.
- **Acceptance**:
  - The list order matches usage; switching to alphabetical reorders it.
  - Choosing an entry navigates the dialog (as R-20).
- **Priority**: Baseline
- **Depends on**: STORE, DRIVE

### R-42 Forget recent folders
- **User need**: I sometimes need to erase the trail, for privacy or tidiness.
- **Behavior**:
  - An action clears the whole list; individual entries can be removed.
  - A setting disables recording altogether.
  - Excluded locations (a user-maintained list) and everything under them are never recorded.
- **Acceptance**:
  - After clearing, the list is empty in all new dialogs and after restart.
  - With recording disabled, using a dialog adds nothing.
  - Folders under an excluded path never appear.
- **Priority**: Next
- **Decision**: moved from Baseline to Next in the user's review of 2026-11-01.
- **Depends on**: STORE

### R-43 Recent folders per app
- **User need**: Folders I use with my photo editor are not the ones I use with my text editor.
- **Behavior**:
  - Recents are recorded with the app that used them, and the strip can show only this app's recents or everyone's.
- **Acceptance**:
  - A folder used only from app A does not appear in A-only view for app B but does appear in the all-apps view.
- **Priority**: Baseline
- **Decision**: moved from Next to Baseline in the user's review of 2026-11-01.
- **Depends on**: READ, STORE

### R-44 Recent files
- **User need**: I want to reopen a file I used recently without finding it.
- **Behavior**:
  - Files chosen in dialogs are recorded and can be listed; choosing one navigates to its folder with the file selected if possible, otherwise just to the folder.
- **Acceptance**:
  - A recorded file appears in the list; choosing it leaves the dialog in its folder.
- **Priority**: Baseline
- **Decision**: moved from Later to Baseline in the user's review of 2026-11-01.
- **Depends on**: READ, STORE, DRIVE

## 6. Per-application starting folders

### R-50 Remember a starting folder for each app
- **User need**: A dialog should start in the place that makes sense for that application, not wherever it defaults to.
- **Behavior**:
  - The user can assign a folder to an app (by window class) from the strip, using the current folder, or from settings.
  - Next time that app shows a dialog, the tool sends it to the assigned folder.
  - A setting per assignment chooses "first dialog after the app starts" or "every dialog".
  - A separate assignment may exist for Open and Save.
  - If the assigned folder is unavailable, the dialog is left alone.
- **Acceptance**:
  - After assigning folder X to app A with "every dialog", each new dialog from A lands in X within a second of appearing.
  - With "first dialog only", the second dialog in the same app session is untouched.
  - Other apps are unaffected.
- **Priority**: Baseline
- **Decision**: moved from Next to Baseline in the user's review of 2026-11-01.
- **Depends on**: STORE, DRIVE, READ, OS

### R-51 Starting folder by file type for Save
- **User need**: Images should be saved to my pictures folder whatever application makes them.
- **Behavior**:
  - An assignment can be limited to files with certain extensions, applying when the Save dialog's file name has a matching extension.
- **Acceptance**:
  - With a rule for `.png`, a Save dialog whose suggested name ends in `.png` starts in the rule's folder.
- **Priority**: Next
- **Decision**: moved from Later to Next in the user's review of 2026-11-01.
- **Depends on**: STORE, DRIVE, READ

### R-52 Start Save dialogs in the document's own folder
- **User need**: When I am editing a file, saving should offer the folder it came from.
- **Behavior**:
  - Optional behavior: if the tool knows or can infer the folder of the document being edited, Save dialogs start there.
  - If it cannot know, the feature does nothing.
- **Acceptance**:
  - When enabled and the document folder is known, a Save dialog starts there.
  - When unknown, behavior is unchanged and no error appears.
- **Priority**: Baseline
- **Decision**: moved from Later to Baseline in the user's review of 2026-11-01.
- **Depends on**: READ, DRIVE, OS

## 7. Search and filtering

### R-60 Fuzzy find among known folders
- **User need**: I want to type a few letters of a folder's name and go.
- **Behavior**:
  - A search box in the strip filters favorites and recent folders as the user types.
  - Matching tolerates missing characters between typed ones; better matches rank first (fewer gaps, matching case, matches at word starts).
  - Enter selects the top result, arrow keys move the highlight, and a numbered shortcut selects a visible result.
  - Searching scans only the tool's known folders, not the whole disk, and returns results with no perceptible delay for lists of several hundred entries.
- **Acceptance**:
  - Typing the initials of a favorite's name lists it first.
  - Result list updates within 100 ms of each keystroke on 500 entries.
  - Enter navigates the dialog to the top result.
- **Priority**: Baseline
- **Depends on**: STORE, DRIVE

### R-61 Search under a folder
- **User need**: When I know roughly where something is, I want to dig for subfolders by name.
- **Behavior**:
  - From a result or the current folder the user can search within that folder's subtree by name, with a depth/time limit and cancellation.
  - Hidden and excluded directories are skipped.
- **Acceptance**:
  - Searching a tree of 50,000 directories returns first results in under one second and can be cancelled instantly.
- **Priority**: Baseline
- **Decision**: moved from Next to Baseline in the user's review of 2026-11-01.
- **Depends on**: FS, DRIVE

### R-63 Context actions on a result
- **User need**: For a found folder I may want something other than navigating.
- **Behavior**:
  - A secondary action on any listed folder offers: navigate dialog, copy path, open in file manager, open in terminal, add to or remove from favorites.
- **Acceptance**:
  - Each action performs the described operation on exactly that folder.
- **Priority**: Baseline
- **Decision**: moved from Next to Baseline in the user's review of 2026-11-01.
- **Depends on**: OS, STORE, DRIVE

## 8. Creating and organizing folders

### R-70 Create a folder in the current folder
- **User need**: I often discover mid-save that the destination does not exist yet.
- **Behavior**:
  - An action asks for a name and creates a directory in the dialog's current folder, then sends the dialog into it (a setting: stay or enter).
  - Invalid names (empty, containing a path separator, already existing) are rejected with a specific message.
  - Nested paths such as `a/b/c` are accepted and created in full.
  - Failure (permissions, read-only volume) is reported and nothing is half-created silently.
- **Acceptance**:
  - Creating `Reports` in `/tmp/x` results in `/tmp/x/Reports` on disk and the dialog showing it.
  - Creating an existing name reports it exists and changes nothing.
  - Creating in a read-only location shows the system error.
- **Priority**: Baseline
- **Depends on**: READ, FS, DRIVE

### R-71 Rename, copy, move, trash the selected item
- **User need**: I sometimes want to tidy a file I just saw in the dialog.
- **Behavior**:
  - Where the selected item is known, actions are offered to rename, duplicate, copy elsewhere, move elsewhere, and send to trash (never permanent deletion by default).
  - Each asks for confirmation if destructive or if it would overwrite.
- **Acceptance**:
  - Each action changes the filesystem as described and the dialog's listing is refreshed or the user is told to refresh.
  - Moving to the trash is recoverable using the desktop's trash.
- **Priority**: Next
- **Decision**: promoted from Later to Baseline in the user's review of 2026-11-01, then set to Next the same day to agree with the answer that defers destructive file actions.
- **Depends on**: READ, FS

## 9. Actions after saving

### R-80 Run a follow-up after a save completes
- **User need**: After saving I often open the file or hand it to another tool.
- **Behavior**:
  - The user can pick, in the strip of a Save dialog, a follow-up: open with the default application, run a user-supplied command with the saved path, or nothing.
  - The follow-up runs only after the file exists, and not at all if the dialog is cancelled.
  - The choice is not remembered unless the user opts in per rule.
- **Acceptance**:
  - Cancelling the dialog runs nothing.
  - Confirming runs the command with the right absolute path once, even with spaces in the name.
- **Priority**: Later
- **Depends on**: READ, OS, STORE

## 10. Dialog window behavior

### R-90 Remember dialog size and position
- **User need**: Dialogs that always open tiny are irritating.
- **Behavior**:
  - Optional: restore the last size and position for each app, or enforce a minimum size.
- **Acceptance**:
  - Resize a dialog, close it, reopen: same geometry. A chosen minimum is respected.
- **Priority**: Baseline
- **Decision**: moved from Later to Baseline in the user's review of 2026-11-01.
- **Depends on**: DRIVE, OS, STORE

## 11. Settings, data and privacy

### R-100 Settings that apply without restart
- **User need**: I want to change behavior and see it right away.
- **Behavior**:
  - All settings are editable in a settings view and in a plain text configuration file; changes apply to the next dialog.
  - Invalid values are reported and ignored without breaking others.
- **Acceptance**:
  - Editing the file by hand and saving takes effect with no restart (or the tool states clearly that a reload is needed).
  - A malformed file never prevents dialogs from working.
- **Priority**: Baseline
- **Depends on**: STORE

### R-101 Data is local, readable and recoverable
- **User need**: I want to know where my lists live and to trust they will not be lost.
- **Behavior**:
  - Data lives in the user's configuration and data directories in readable formats.
  - Writes are atomic; a corrupt file is set aside and the tool starts clean with a message.
  - The tool sends nothing over the network.
- **Acceptance**:
  - Killing the tool mid-write leaves a valid previous file.
  - Network traffic from the tool during use is zero.
- **Priority**: Baseline
- **Depends on**: STORE

### R-102 Pause the tool
- **User need**: I sometimes want chrome off temporarily (screen sharing, troubleshooting).
- **Behavior**:
  - A command and a shortcut toggle the tool on and off; while off, nothing is shown, recorded or moved.
- **Acceptance**:
  - While paused, dialogs show no strip and the recent list does not grow.
- **Priority**: Baseline
- **Decision**: moved from Next to Baseline in the user's review of 2026-11-01.
- **Depends on**: OS, STORE

### R-103 Appearance follows the desktop
- **User need**: The strip should look like it belongs.
- **Behavior**:
  - Colors and fonts follow the desktop's current theme, including changes while running; icon-only or icon-and-text labels are selectable.
- **Acceptance**:
  - Switching the desktop theme changes the strip on the next dialog or sooner.
- **Priority**: Baseline
- **Decision**: moved from Next to Baseline in the user's review of 2026-11-01.
- **Depends on**: OS, STORE

### R-104 Fail visibly, never silently
- **User need**: If something goes wrong I want to know, not wonder.
- **Behavior**:
  - Failures to read the dialog state, move the dialog or write data produce a short visible message in the strip and a log entry.
  - A log file records events at a selectable verbosity.
- **Acceptance**:
  - Forcing each failure produces both a message and a log line.
- **Priority**: Baseline
- **Depends on**: NONE

### R-105 Preview the selected picture or video
- **User need**: Before I pick an image or a video in a dialog, I want to see it.
- **Behavior**:
  - While the item selected in the dialog is an image or a video, a panel under the dialog shows the picture, or a still from the video, with its name, size and date.
  - Anything else selected, or nothing, hides the panel. It has no on and off control of its own beyond the settings; it is always available.
  - Moving through a folder with the arrow keys does not make the panel flicker.
- **Acceptance**:
  - Selecting an image shows it under the dialog; selecting a text file or a folder hides it.
  - The panel never takes the keyboard when it appears.
- **Priority**: Baseline
- **Decision**: added by the maintainer on 2026-11-03 ("it just shows when an image/video is selected, that way it can always be on").
- **Depends on**: reading the dialog's selection (see `docs/adapters.md`, Selection)

## Not requirements

- Menu-bar or status-area menu with its own copy of every list: the panel is tied to dialogs; a launcher or file manager covers the rest.
- Launcher-style whole-computer search and application launching: out of scope for dialog chrome.
- A temporary holding area for dragging files between windows: a general file-manager feature that does not depend on a dialog.
- Editing tags, comments, ownership, permissions or timestamps of files: file-manager territory and platform-specific.
- Built-in automation languages and workflow engines: a user command hook (R-80) is enough.
- Self-update machinery, update checks and licensing flows: handled by the package manager.
- Mouse-over highlighting and click-through capture of other windows to choose a folder: needs input interception that conflicts with a Wayland session's security model; R-25 offers a list-based substitute.
- Screen-capture based masking of the tool's own actions: unnecessary when it does not drive the dialog visibly.
- Accessibility modes that remove controls from the dialog: the strip must instead be accessible (R-05).
- Mirroring a cloud-sync client's own activity: out of scope; users can add synced folders as favorites.
- Support-bundle and performance-test buttons: replaced by the log in R-104.

## Coverage

Where the first build stands against each Baseline requirement. **Met** means every acceptance line is covered by a named test or a recorded live run. **Partly met** names what was not exercised. A real mouse click and typing in a field were never driven unattended, so anything that depends on them is at best partly met. Live runs are `spike/proxy_strip_check2.py` (two panels docked, focus kept, follows a move, cleans up: Flea, GTK and Strata, two runs each), `spike/proxy_record_check.py` (Flea and GTK), and `ofolder live-check` (adapters: Flea 13 of 14, GTK 3 of 3, Strata 3 of 3).

| ID | Status | Evidence, and what is not covered |
|---|---|---|
| R-01 strip appears with the dialog | Partly met | Live on all three dialogs. Not covered: two dialogs open at once; the under-one-second start was not timed on the final build |
| R-02 two panels, one each side | Met | `geometry` placement tests (split, packed, overlap); live docking on all three. The `swap_sides` setting is read but not applied, which the maintainer deferred |
| R-04 start with the session | Partly met | `packaging/` holds the portal and D-Bus activation files; not installed, because registering the proxy is the user's decision |
| R-05 keyboard-only operation | Partly met | Built 2026-11-03 as shortcuts: every rail command, and the settings window, can have a key combination, bound in Hyprland while a dialog is open (`shortcuts`, `keybind`, `keys` tests). Live on Flea: combinations recorded in the Shortcuts pane were bound and removed with the dialog, and the command a binding runs (`ofolder do`) opened menus, copied the path, moved the dialog to its parent and back. Find and New folder take the keyboard when opened this way. Not covered: an actual key press reaching a binding (synthetic keys from `wtype` do not trigger Hyprland bindings, and no other key injector is available); choosing a row in a list menu still needs the mouse; there is no single shortcut that moves focus into a rail |
| R-06 large lists | Partly met | Row cap with "Show more", off-thread folder reads, view-model tests. Not covered: a hung mount, a real very large folder |
| R-11 copy path | Met | Live click on the real panel: the clipboard held the dialog's folder, on Flea, GTK and Strata; model tests cover the unknown-folder refusal |
| R-12 open in file manager | Partly met | `launch` tests with fake desktop entries; spawn test. Not covered: a click on the real button |
| R-13 open in terminal | Met | Live click: a terminal opened with the dialog's folder as its working directory, on Flea, GTK and Strata (then closed); `launch` tests |
| R-20 send the dialog to a folder | Met | `ofolder live-check` on Flea, GTK and Strata, and live panel clicks that moved the dialog (see R-23) |
| R-22 standard places and drives | Partly met | Parsers tested on sample files, rows seen in a demo screenshot. Not covered: a click; a live drive plug or unplug |
| R-23 parent, back, forward | Partly met | Live through the real portal frontend on Flea and GTK: Downloads, then Parent, then Back landed in Downloads, and a stale poll no longer rewrites the history (`places` and `model` tests). Not confirmed on Strata, whose dialog the test could not end with a key |
| R-24 browse subfolders | Partly met | `browse` and `expand` tests; nesting seen in a screenshot. Not covered: a click on the arrow |
| R-25 folders of open file managers | Partly met | Flea 0.4 and 0.3 windows are listed in the Places menu and Find, each from its own window; checked end to end on all four backends. Strata cannot report its main windows' folders exactly, so it is left out (`docs/adapters.md`) |
| R-30 favorites list | Partly met | Favorites tests; missing-folder badge seen. Not covered: clicks in the final layout |
| R-31 add current folder | Met | Live click: the button flipped to Unfavorite and the favorites file held the dialog's folder, on Flea, GTK and Strata |
| R-32 edit, reorder, remove | Partly met | Menu actions tested; not clicked |
| R-33 shortcut per favorite | Partly met | A shortcut opens the Favorites menu (and any other rail command); a combination another command holds asks to swap or clear, and one the user's own Hyprland configuration binds is refused. Not built: a combination for one particular favorite folder. Differs from the acceptance: while a dialog is open the combination is taken from every window, not only the dialog, because Hyprland bindings are global; it is released when the dialog closes |
| R-40 remember folders used | Met | Live through the real portal frontend on Flea, GTK and Strata: a confirmed save records the folder and file, a cancel records nothing and creates nothing |
| R-41 choose from recent folders | Partly met | Section seen in a screenshot, ranking tested. Not covered: a click |
| R-43 recents per app | Partly met | Live: recents are tagged `class:<window class>` of the app that asked, through the real frontend. On this desktop apps send no id. Not covered: a click on This app |
| R-44 recent files | Partly met | Files recorded live. Not covered: a click on a recent file |
| R-50 starting folder per app | Met | Live through the real portal frontend on Flea, GTK and Strata: a `class:` start-folder rule seeded the dialog for an app that named no folder; hook and private-bus tests show it never overrides one the app named |
| R-52 the document's own folder | Partly met | The app's folder is always kept, so it works for apps that send one. Nothing more is possible for the rest |
| R-60 fuzzy find | Partly met | Live on Flea and GTK: typing into the find box then Enter moved the dialog (`fuzzy`, `finder` tests). Not covered: Up, Down, Escape, and Strata |
| R-61 search under a folder | Partly met | `search` tests including cancel, hidden folders, symlinks, and an ignored 50k-folder performance test. Not covered: Stop pressed live |
| R-63 actions on a result | Partly met | Menu tests; not clicked |
| R-70 new folder | Partly met | Live on Flea, GTK and Strata: typing a name and Enter in the field created exactly that folder in the dialog's scratch folder. Not confirmed: the dialog then moving into it |
| R-90 dialog size and position | Partly met | `dialog_memory` tests and wired through the float rule. Not covered: a visible jump when the remembered place is applied |
| R-100 settings without restart | Partly met | Read per dialog, lenient parsing with warnings (`settings` tests). The settings window edits the General settings and the rail layout and saves at once (`settings_write`, `rails` tests; live: toggles, the recents count, a command field, the rail layout and reset written to a scratch file with its comments kept, and the rails redrawn at once). Start folders, recent exclusions and excluded applications are edited in the file only |
| R-101 data local and recoverable | Partly met | Plain TOML and JSON under the user's folders, atomic writes, a corrupt file renamed aside; the settings window keeps comments and unknown keys and leaves a file it cannot parse alone (`settings_write` tests). The settings keys are listed in `packaging/README.md`; the JSON formats are not yet written up for users |
| R-102 pause | Partly met | `ofolder pause` live: with it on, a real dialog opened with no panels and finished normally, on Flea, GTK and Strata; `resume` restores them. No keyboard shortcut |
| R-103 appearance follows the desktop | Partly met | The active theme's `colors.toml` is read per dialog with a default fallback (`theme` tests), mapped in one place (8 `colors` tests); in live runs the panels took the desktop theme's accent. Not covered: a live theme change, and a check against more themes |
| R-104 fail visibly | Partly met | Errors show in a message window beside the right rail, sized to their text, held while the pointer rests on them, and on stderr; every message is logged to `~/.local/state/ofolder/messages.log` (last 200 lines) and listed in the settings window's Messages pane (`messages` and `model` tests; live: a launcher failure shown in full beside the gear, logged, and listed). Not covered: a selectable verbosity, and failures to read the dialog state, which show as "folder unknown" rather than a message |
| R-105 preview the selected picture or video | Partly met | Live on Flea: PNG, JPEG, GIF and an MP4 still shown under the dialog; a text file hid it; three fast Down presses over two images showed it once, after settling; it followed a dialog move and hid when no room was left below; the settings switch and the `preview` shortcut hid and showed it; the active window did not change when it appeared (`preview` tests in core and strip). Not covered: GTK and Strata, a click on the caption (synthetic clicks did not reach it), SVG, WebP, BMP and TIFF files |

The panels became icon rails with menu windows on 2026-11-02 (see `docs/decisions.md`). That layout was checked live on Flea only, through the real portal frontend with real clicks: menus open beside their rail at the icon and float; Places and Favorites rows, a Find result, Back and Parent each moved the dialog and closed the menu; Find took the keyboard, Escape cleared and then closed it; New folder created a folder in a scratch folder and moved into it; Copy path, the favorite toggle and the Terminal launcher acted at once; clicking a menu's icon again closed it; giving the dialog the keyboard closed the open menu. GTK and Strata were not rerun with the rails. The click offsets in `spike/real_portal_check.py` belong to the old wide panels.

The live sweep is `spike/real_portal_check.py`. It temporarily registers ofolder as the first file-chooser backend, asks for a Save dialog from a small GTK test app through the real portal frontend, drives the panels with synthesized mouse clicks and typed keys (typing only after the target window is verified active), and then restores the portal configuration and checks it by hash. One finding from it was fixed: Flea's helper runs from the home folder, so a dialog showing home read as "unknown" until the working-directory exclusion was removed from the descriptor reader.

R-10 and R-21 were dropped by the maintainer. R-71 is Next.

## Open questions

1. Which dialogs are in scope for first release: only portal-provided choosers, or also toolkit-native ones?
2. If the dialog's current folder cannot always be read, should Baseline features that need it (R-10, R-11, R-31, R-40) degrade silently, show "unknown", or block the release?
3. Should "recent folders" record folders when a dialog is only browsed, or only when a file is actually opened or saved?
4. Are recents global, per app, or both by default? Are recents shared across favorite sets?
5. Should a favorites jump preserve the typed file name in Save dialogs even if it requires extra effort, or is best-effort acceptable?
6. What is the acceptable delay between a dialog appearing and the strip appearing?
7. Which file manager and terminal are defaults, and should they be auto-detected from the desktop's configuration?
8. Should the starting-folder feature (R-50) override a folder the application itself chose deliberately, such as the document's own folder?
9. Is the strip allowed to take keyboard focus automatically in any case (for example when opened by a shortcut), and which global shortcuts are acceptable on this desktop?
10. What is the data format and location commitment for lists and settings (plain text, TOML, JSON), and is it a stable public interface?
11. Should destructive file actions (R-71) exist at all in a first-party strip, or should the tool only navigate and create folders?
12. How should multiple monitors and floating versus tiled dialog placement affect docking?
