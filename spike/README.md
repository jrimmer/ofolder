# The spike

Before any of ofolder was written in Rust, a throwaway Python spike asked one question and kept asking
it until the answer was boring: **can something outside a file dialog dock beside it, read what it shows
and steer it, without changing the dialog?** There is no supported API for that, and Wayland will not let
one process embed another's window, so nothing could be assumed. Each idea was a small script that tried
it against the real dialog, wrote a log, and said plainly whether it worked.

The findings were then distilled into [`docs/adapters.md`](../docs/adapters.md), which is what the Rust
code is built on. This folder is the evidence behind that document, kept so the claims can be checked and
the method reused.

## How it was done

1. **Ask one narrow question per script.** "Can a strip sit docked to the picker and follow it?"
   "Does a respawn change its folder?" "Can the GTK dialog be moved with no keystrokes at all?"
2. **Try it on the real thing**, with a scratch folder and a verified target window, and write down
   what happened, including the failures.
3. **When a result is surprising, repeat it before believing it.** `gtk_probe5.py` moved the GTK dialog
   and then passed and failed in turns; `gtk_probe6.py` exists only to find out whether that was timing
   or a swallowed first keypress. It was the swallowed keypress, and the adapter now accounts for it.
4. **Promote what survives.** The probes became adapters, the adapters became the portal proxy, and the
   proxy checks (`proxy_*_check.py`, `real_portal_check.py`) proved the whole path end to end, first
   through the proxy and then through the real portal frontend. Those are the ancestors of the `e2e/` suite.

## What is here

| Script | Question |
| --- | --- |
| `strip_spike.py`, `strip_hardened.py`, `strip_multi.py` | Can a strip dock beside Flea's picker, follow it and steer it? Then: only type into a verified window; then: one strip over Flea, GTK and Strata behind a common adapter interface |
| `probe_folder.sh`, `probe_folder2.py` | Where can the strip read the picker's current folder from? (The directories its helper process holds open.) |
| `gtk_probe1.py` to `gtk_probe6.py`, `gtk_call.py`, `gtk_diag.py` | What can be seen of the GTK portal dialog from outside, over AT-SPI, and which ways of moving it work |
| `strata_probe1.py` to `strata_probe3.py` | The same for Strata |
| `portal_call.py`, `app_id_probe.py` | Calling a backend's file-chooser directly; what application id the portal frontend passes along |
| `proxy_gtk_check.py`, `proxy_record_check.py`, `proxy_strip_check.py`, `proxy_strip_check2.py` | The ofolder proxy between a client and the real dialog: answers come back, confirmed folders are recorded, the strip appears and cleans up |
| `real_portal_check.py` | The same through the real portal frontend, with real clicks |
| `watch_first_frame.py` | How the strip's own window settles in its first moments (floating, position and size, sampled every 15 ms) |

`runs/<name>/` holds the log of each run (and, for three of them, a screenshot). The screenshots in
`runs/multi-*/1-docked.png` show one design docked beside Flea, GTK and Strata; the file lists were taken
from a real home folder and are blurred.

## Before you run anything here

These scripts are evidence, not tools. They open real dialogs, send keystrokes with `wtype`, and several
assume the machine and versions they were written on (Hyprland 0.56, Flea 0.3.12 and 0.4, the GTK portal
1.15, Strata 1.1.1). `real_portal_check.py` temporarily registers ofolder as the system's file-chooser backend
and rewrites your portal configuration (it keeps a backup and restores it, but read it first). The
supported way to exercise the real dialogs is `bin/e2e`, which does it in a private nested Hyprland and touches
nothing of yours.

Some evidence was reduced before publishing: 52 screenshots of a real desktop were dropped, and user names,
host names and the names of unrelated folders were replaced in the logs.
