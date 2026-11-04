# Working on ofolder

ofolder docks a strip of chrome (favorites, new folder, go-to-folder) beside the
save and open dialogs of Flea, the GTK portal and Strata, and drives them. There
is no supported API for that, so each dialog gets an adapter that says honestly
what it can do. `docs/adapters.md` has what was measured and the hazards seen;
it wins where this file and it differ. `spike/` is the throwaway Python spike
that produced those findings; `spike/runs/` is its evidence.

## Rules

1. **No attribution.** Commits, comments, docs, and issues carry no authorship or tool attribution lines. Add no `Co-Authored-By` trailers.
2. **Never type into a window you have not verified is the target.** `wtype` goes to whatever has focus. Check the dialog is the active window before each step, and refuse when it is not.
3. **Never guess a folder.** An adapter reads the dialog's folder or returns `None`. "New folder" and every other write refuse on `None`.
4. **Leave the system alone.** Do not change `~/.config/xdg-desktop-portal`, Hyprland configuration, or Flea, GTK or Strata settings, and do not use `sudo`. A test that must enable Strata's portal backs the config up first, restores it, and proves the restore with a hash. Do not kill processes you did not start.
5. **Tests never touch real folders.** Fixtures live in temporary folders. A check that opens a dialog uses a scratch folder, never `~`.
6. **Git.** Work on your own branch. Never push to `main`, never force-push, and read `git remote -v` out loud before any push. Do not push without being told to. Commit messages are short and imperative.
7. **Dependencies.** Pin gpui 0.2.2 from crates.io and keep `libc` at 0.2.186. Build with `--locked`. Do not depend on Zed's GPL or AGPL crates, and do not copy code from them.
8. **Gate.** `bin/check` must pass before you report done. A change to what a dialog shows or does also needs `bin/e2e` to pass (it needs a display; `docs/e2e.md`).
9. **Never weaken a check to make it pass.** Do not delete or loosen a test or edit expected output to match. If a check is wrong, say so and why.
10. **Prove it.** A claim in a report needs the command you ran and its output, or an artifact path such as a screenshot or a log.

## Rust

- Edition 2024. `rustfmt` defaults. `clippy -D warnings`.
- Errors use `thiserror` in libraries and `anyhow` only in `main`.
- No `unwrap()` or `expect()` outside tests, unless a comment states why it cannot fail.
- Comments explain a non-obvious why, in full sentences. Do not narrate what the code does.
- Name tests after the behaviour they check.
- `ofolder-core` stays free of gpui and of any D-Bus or accessibility crate; those live behind the `Adapter` trait in their own crates.
