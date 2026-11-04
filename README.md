# ofolder

Chrome for the save and open dialogs on Omarchy: a strip docked beside the
dialog with favorites, "new folder" and go-to-folder. It decorates Flea, the GTK portal dialog and Strata without changing
any of them.

Wayland cannot embed another process's window, and none of these dialogs has a
plugin API, so ofolder reads and drives each dialog from outside through an
adapter that says honestly what it can do.

- `docs/adapters.md`: what was measured about each dialog, and the hazards.
- `crates/core`: the adapter contract and the pure, tested logic (no UI, no D-Bus).
- `crates/strip`: the gpui strip.
- `crates/app`: the `ofolder` command.
- `e2e/`: the end-to-end suite behind `bin/e2e`, see `docs/e2e.md`.
- `spike/`: the throwaway Python spike behind the findings, with its evidence in `spike/runs/`.

```
bin/check              # fmt, clippy -D warnings, tests, architecture guard
bin/e2e                # real dialogs in a private nested Hyprland (docs/e2e.md)
bin/install            # install the release build safely, keeping the previous one (--rollback)
bin/canary             # what changed in Flea, Strata, the portal or Hyprland since the last look
ofolder doctor         # check how ofolder is set up; ofolder repair puts it right
cargo run -p ofolder -- adapters     # which installed file choosers can be decorated
cargo run -p ofolder -- strip-demo   # open a sample strip window
```

Status: the portal proxy, the Flea, GTK and Strata adapters and the rails are in Rust and
proven against the real dialogs (see `docs/adapters.md` and the coverage table in
`docs/requirements.md`). The Python spike in `spike/` is kept only as evidence and as the live
test harness.
