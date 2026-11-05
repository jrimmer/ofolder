# Releasing ofolder

A release is cut from a Git tag on the public GitHub repository (`jrimmer/ofolder`).
Pushing `vX.Y.Z` starts `.github/workflows/release.yml`, which builds the tarball,
creates the release as a draft, signs it, and publishes it last. Nothing publishes
from your machine: `scripts/make-release-tarball` only writes files into a folder.

The asset names are the contract praeco and `install.sh` rely on, so keep them exactly:

| Asset | Used for |
| --- | --- |
| `ofolder-linux-x86_64.tar.gz` | the tarball `install.sh` and praeco install (a `tarball` install kind) |
| `ofolder-linux-x86_64.tar.gz.sha256` | its checksum, read by `install.sh` and praeco |
| `install.sh` | the one-line installer (`curl -fsSL .../releases/latest/download/install.sh \| bash`) |
| `SHA256SUMS` | checksums over every file in the release folder (the tarball, its `.sha256`, `install.sh`) |
| `packslip.sigstore.json` | the signature over the tarball, attached by the workflow just before publishing |

The tarball holds `ofolder-<version>/bin/ofolder` and
`ofolder-<version>/lib/ofolder/praeco`. `install.sh` and praeco strip the
`ofolder-<version>/` prefix and install the tree under `~/.local`, which is where
ofolder looks for praeco. `install.sh` also keeps the program it replaces as
`~/.local/bin/ofolder.previous`, as `bin/install` does.

## Cut a release

1. **Changelog.** In `CHANGELOG.md`, add a section `## X.Y.Z (YYYY-MM-DD)` with
   the changes. The release notes are the text above the `## Install` heading in
   that section, and praeco shows only that text as "what's new". The workflow
   drops the section's own heading (the release title names the version) and
   keeps the rest; a section with no `## Install` line gets the one-line install
   steps under one, so you only write an `## Install` heading yourself when the
   release needs special steps:

   ```markdown
   ## 0.2.0 (2026-12-01)

   - Fixed the portal check on Hyprland 0.50.

   ## Install

   Install with one line (this release needs nothing special):

       curl -fsSL https://github.com/jrimmer/ofolder/releases/latest/download/install.sh | bash
   ```

   Keep `## Install` as a heading line exactly, and keep the install steps below
   it. A level-2 heading that is not `## Install` ends the section.

2. **Version.** Set `version` under `[workspace.package]` in `Cargo.toml` to
   `X.Y.Z`, the same number as the changelog heading. Then check that the lock
   still builds: `cargo build --release --locked -p ofolder`. If it fails, the
   lock needs updating first, and that is a separate commit.

3. **Gate, locally.** Run `bin/check`, and under the shared lock
   `flock -w 7200 /tmp/ofe2e.lock bin/e2e` (see `docs/e2e.md`). `bin/check` also runs
   in GitHub Actions (`.github/workflows/check.yml`) on every push and pull
   request, but the e2e suite drives real dialogs in a nested Hyprland, needs a
   display, and cannot run there: it stays a local gate before tagging. `bin/check`
   includes the tarball test (`scripts/test-release-tarball`) and the installer
   test (`scripts/test-install`), neither of which needs the network.

4. **Try the tarball (optional, recommended).** `scripts/make-release-tarball OUTDIR`
   builds the binary, fetches the pinned praeco (`scripts/get-praeco`), assembles the
   archive and writes `ofolder-linux-x86_64.tar.gz.sha256` and `SHA256SUMS`. It
   publishes nothing. `OUTDIR` is created if needed, and it should be a fresh
   folder. Copy `install.sh` into `OUTDIR` first if you want `SHA256SUMS` to cover it
   (the workflow does). Then extract it into a new temporary folder, never the real
   `~/.local`, and run the two binaries from there:

   ```sh
   T=$(mktemp -d)
   tar -xzf OUTDIR/ofolder-linux-x86_64.tar.gz -C "$T"
   "$T"/ofolder-X.Y.Z/bin/ofolder version
   "$T"/ofolder-X.Y.Z/lib/ofolder/praeco --version   # must print: praeco 0.4.0
   # The portal must start (exit 0). It needs a session bus that knows its name, so run it
   # in a private one: dbus-run-session -- "$T"/ofolder-X.Y.Z/bin/ofolder portal --check
   (cd OUTDIR && sha256sum -c ofolder-linux-x86_64.tar.gz.sha256 SHA256SUMS)
   rm -rf "$T"
   ```

   To rehearse `install.sh` itself without the network and without your real
   home, point it at that folder and a private home:

   ```sh
   H=$(mktemp -d)
   HOME=$H OFOLDER_NO_SYSTEMD=1 OFOLDER_RELEASE_URL=file://$PWD/OUTDIR bash install.sh --no-setup
   rm -rf "$H"
   ```

5. **Tag and push.** Commit the version bump and the changelog section, then
   push the tag. The tag must be `v` plus exactly Cargo.toml's version; a
   mismatch, or a version with no changelog section, fails `check-version`
   before anything is built.

   ```sh
   git tag vX.Y.Z
   git push origin vX.Y.Z
   ```

6. **Wait for the workflow, then look.** It builds the tarball, creates the
   draft, signs it, attaches `packslip.sigstore.json` and publishes. Open the
   release page and check the five assets above, the notes, and that it is not a
   draft. Never publish a draft by hand: a release without its signature blocks
   every update until it appears.

7. **Update rehearsal.** Before the release is announced, check that an installed
   older release updates to the new one. Work in a temporary root only:

   - Extract the previous release's tarball into a new temporary folder, which
     stands in for `~/.local`, and use its own `lib/ofolder/praeco`.
   - Run `praeco check SOURCE --current <previous version> --exe <temp>/bin/ofolder`
     where `SOURCE` is the release source. Set `OFOLDER_UPDATE_SOURCE` to point at
     the new release, and `OFOLDER_PRAECO` to the praeco binary you are testing.
     Check that praeco reports the new version.
   - Optionally run `praeco install` against the same temporary root, then run the
     installed `bin/ofolder version`.

   Never run this against the real `~/.local`, the real desktop or the user's
   `~/.config`. Those are for the maintainer to test by hand.

To re-run a release, either re-run the failed workflow, or start the **Release**
workflow manually and give it the existing tag. While the release is still a draft,
a re-run uploads the assets again with `--clobber`, refreshes the notes and signs
again. Once it is published, a re-run fails on purpose: the signature describes
the files that were published, so a changed release needs a new tag.

## What the workflows do

`.github/workflows/release.yml` has five jobs:

- **check-version**: checks out the tag and fails unless the tag without its leading
  `v` equals the workspace version in `Cargo.toml` and `CHANGELOG.md` has a section
  for it.
- **build**: on `ubuntu-24.04` (x86_64) installs the system packages the build needs
  (gpui links libxkbcommon, libxkbcommon-x11 and libxcb; fontconfig, freetype,
  wayland, clang and cmake are for the crates that build against them), runs the two
  no-network tests, copies `install.sh` into the output folder, runs
  `scripts/make-release-tarball`, checks the checksums and the extracted binaries,
  and uploads the folder as the artifact `ofolder-linux-x86_64`.
- **release**: downloads the artifact, checks every expected asset, writes the notes
  from the changelog, and creates the GitHub release as a **draft** with
  `gh release create --draft` (or `gh release upload --clobber` when a draft already
  exists). It fails on a release that is already published.
- **packslip**: signs the tarball with [packslip](https://github.com/jdx/packslip),
  keyless, with the workflow's own identity, and uploads `packslip.sigstore.json` as
  a workflow artifact. It has no write access to the release.
- **publish**: attaches `packslip.sigstore.json` to the draft and publishes it. If
  signing fails, nothing is published and the draft stays invisible.

`.github/workflows/check.yml` runs `bin/check` on every push and pull request. It
is a convenience gate, not a replacement for the local one (no e2e, see above).

Neither workflow can be run locally. The first release candidate tag is their
test: see "Before the first public release".

## Signing

Releases are signed so that praeco, the in-app updater, can check a download
against a signature made by this workflow instead of a checksum that sits beside
the file. There is no key to keep. The first time praeco sees a signed ofolder
release it remembers the repository and the signing workflow, and from then on:

- **A release without a valid signature is refused.** That is why the release stays
  a draft until the last job.
- **Only files the signature lists are installed.** It lists the tarball.
  `install.sh`, the checksums and anything else are not on the list and praeco will
  not install them; do not add files to a published release by hand.
- **Changing the signing workflow's identity** (renaming `release.yml`, moving the
  repository) makes praeco refuse the next release until each user deletes ofolder's
  entry from `~/.config/praeco/signers.json`. Do not do it casually.
- **The signing action is pinned to a commit**, because its code decides what is
  signed. Raise the pin on purpose, after reading what changed.

## Before the first public release

These were written without being able to run GitHub Actions or to see the public
repository, so the first release candidate tag (for example `v0.1.0-rc1`, deleted
afterwards together with its draft) must confirm:

- The system package names in both workflows install on `ubuntu-24.04` and are
  enough for `cargo build --release --locked -p ofolder` (the names are assumptions).
- `check.yml` passes: `bin/check` runs there without a display, including the guard
  test (it needs `dbus-daemon`) and the new uninstall and installer tests.
- A tag whose version has no changelog section fails in `check-version`; a good one
  gets through.
- The draft has exactly the five expected assets, the notes stop at `## Install`
  as praeco shows them, and `packslip.sigstore.json` is attached before it goes public.
- On a throwaway machine or a private home, the published one-liner installs, and
  `--uninstall` removes it again.
- The repository is public and the release is signed under the workflow's identity;
  praeco's first sight of it records that identity.
- ofolder's built-in update source is still the user's Forgejo (`DEFAULT_FORGEJO_BASE` in
  `crates/core/src/update.rs`). Until it is changed to `github jrimmer/ofolder`, an installed
  ofolder looks for updates on the Forgejo release, not on GitHub.

## Environment variables

The update notice reads these variables. They are documented in
`docs/updates.md`, which has the full list:

- `OFOLDER_UPDATE_SOURCE`: where the update check looks for releases.
- `OFOLDER_PRAECO`: the praeco binary to use instead of the installed one.
- `OFOLDER_PRETEND_VERSION`: the version the update check reports as running,
  so an older install can be tested against a newer release.
- `OFOLDER_UPDATE_CHECK_DELAY`: seconds before the first check. Setting it also
  forces the check, which is what the rehearsal needs.

`install.sh` reads `OFOLDER_RELEASE_URL` (where the release files come from; the
tests use a `file://` folder). `OFOLDER_NO_SYSTEMD=1` keeps `ofolder repair` and
`ofolder uninstall` from touching the user's systemd units or restarting the portal.

## Keeping releases readable by older praeco

Every release is installed by the praeco the user already has, not the one in
the new release. So:

- Keep the file names and kinds above. The tarball must stay a `*.tar.gz` with
  the `ofolder-<version>/bin/ofolder` layout.
- Give other systems' downloads extensions praeco does not match for Linux, so
  they are not installed over the tarball.
- Keep the `## Install` heading in every release's notes (the workflow adds one when
  the changelog has none).
- Do not change the praeco pin in `scripts/get-praeco` in a release unless the
  new praeco has been tested; the pin is the checksum of the binary that is
  shipped.
