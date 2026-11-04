# Releasing ofolder

A release is a tarball with the ofolder binary and the praeco update tool
beside it, plus checksums and release notes. Nothing here publishes on its own:
uploading to the Forgejo release is a manual step that the maintainer decides on.

The asset names are the contract praeco relies on, so keep them exactly:

| Asset | Used for |
| --- | --- |
| `ofolder-linux-x86_64.tar.gz` | the tarball praeco installs (a `tarball` install kind) |
| `ofolder-linux-x86_64.tar.gz.sha256` | its checksum, read by praeco |
| `SHA256SUMS` | checksums over every file in the release |

The tarball holds `ofolder-<version>/bin/ofolder` and
`ofolder-<version>/lib/ofolder/praeco`. praeco strips the `ofolder-<version>/`
prefix and installs the tree under `~/.local`, which is where ofolder looks for
praeco.

## Checklist

1. **Changelog.** In `CHANGELOG.md`, add a section `## X.Y.Z (YYYY-MM-DD)` with
   the changes. The release notes are the text above the `## Install` heading in
   that section, and praeco shows only that text as "what's new", so put the
   install steps below the heading, for example:

   ```markdown
   ## 0.2.0 (2026-12-01)

   - Fixed the portal check on Hyprland 0.50.

   ## Install

   Download `ofolder-linux-x86_64.tar.gz`, then extract it into `~/.local`:

       tar -xzf ofolder-linux-x86_64.tar.gz -C ~/.local --strip-components=1

   Or build from source with `bin/install`.
   ```

   Keep `## Install` as a heading line exactly, and keep the install steps below
   it. Write the release notes to a file in the output folder before step 4, so
   that `SHA256SUMS` covers them.

2. **Version.** Set `version` under `[workspace.package]` in `Cargo.toml` to
   `X.Y.Z`, the same number as the changelog heading. Then check that the lock
   still builds: `cargo build --release --locked -p ofolder`. If it fails, the
   lock needs updating first, and that is a separate commit.

3. **Gate.** Run `bin/check` and fix anything it reports. It includes the release
   tarball test, which needs no network.

4. **Build the tarball.** `scripts/make-release-tarball OUTDIR` builds the
   binary, fetches the pinned praeco (`scripts/get-praeco`), assembles the
   archive and writes `ofolder-linux-x86_64.tar.gz.sha256` and `SHA256SUMS`. It
   publishes nothing. `OUTDIR` is created if needed, and it should be a fresh
   folder for each release.

5. **Verify the tarball.** Extract it into a new temporary folder, never the
   real `~/.local`, and run the two binaries from there:

   ```sh
   T=$(mktemp -d)
   tar -xzf OUTDIR/ofolder-linux-x86_64.tar.gz -C "$T"
   "$T"/ofolder-X.Y.Z/bin/ofolder version
   "$T"/ofolder-X.Y.Z/lib/ofolder/praeco --version   # must print: praeco 0.4.0
   (cd OUTDIR && sha256sum -c ofolder-linux-x86_64.tar.gz.sha256 SHA256SUMS)
   rm -rf "$T"
   ```

   The `sha256sum -c` line checks the archive and the release files. A failure
   there means a file changed after step 4: rerun step 4 rather than editing
   the output by hand.

6. **Publish (manual).** When the maintainer says so, upload to the Forgejo release
   for the tag `vX.Y.Z`: the tarball, its `.sha256`, `SHA256SUMS` and the notes.
   Do not upload a tarball that did not pass step 5. Tag the commit that has the
   version bump and the changelog section.

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

## Environment variables

The update notice reads these variables. They are documented in
`docs/updates.md`, which has the full list:

- `OFOLDER_UPDATE_SOURCE`: where the update check looks for releases.
- `OFOLDER_PRAECO`: the praeco binary to use instead of the installed one.
- `OFOLDER_PRETEND_VERSION`: the version the update check reports as running,
  so an older install can be tested against a newer release.
- `OFOLDER_UPDATE_CHECK_DELAY`: seconds before the first check. Setting it also
  forces the check, which is what the rehearsal needs.

## Keeping releases readable by older praeco

Every release is installed by the praeco the user already has, not the one in
the new release. So:

- Keep the file names and kinds above. The tarball must stay a `*.tar.gz` with
  the `ofolder-<version>/bin/ofolder` layout.
- Give other systems' downloads extensions praeco does not match for Linux, so
  they are not installed over the tarball.
- Keep the `## Install` heading in every release's notes.
- Do not change the praeco pin in `scripts/get-praeco` in a release unless the
  new praeco has been tested; the pin is the checksum of the binary that is
  shipped.
