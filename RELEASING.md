# Releasing bukio

For the maintainer. The artifacts are the product: `install.sh`, `bukio update`
and `cargo binstall` all consume what the release workflow publishes, so the
names and the checksum file are a contract — change them in all four places or
none (workflow, installer, updater, README).

## The one rule

**A release that existing Node users may still open must not change the schema.**
The Rust and Node versions share migrations; a book written by 0.17 opens in
0.18 and vice versa, which is what makes rollback safe. Add a migration and you
silently break every older binary out there. If a release genuinely needs one,
it is a breaking release: say so in the changelog and in the release notes.

## Before the tag

1. **Version.** `Cargo.toml` only — `bukio --version` reads
   `CARGO_PKG_VERSION`, and a test fails if the two ever disagree. Also bump the
   version badge in `README.md`.
2. **Changelog.** Move `[Unreleased]` in `CHANGELOG.md` under a
   `## [0.18.0] — YYYY-MM-DD` heading, written for a user rather than a
   reviewer: what changed about *using* it, what to do about it.
3. **Report last.** Run `scripts/testreport.sh` as the final commit before
   tagging, so the committed report and badge describe the tagged tree.
4. **Suite green**, all of it: `cargo test --release --no-fail-fast`.
5. **Cross-version test green.** `tests/cross_version.rs` opens a book the Node
   version wrote (fixture committed under `tests/fixtures/`) and asserts the
   reports match. See the header of that test for how to regenerate the fixture.
6. **Installer smoke test**, twice: against a locally-built artifact (see below),
   and on a clean container of the oldest supported distro. The second one is the
   only thing that catches a glibc or libc mismatch before users do.

## Tag it

```sh
git tag v0.18.0
git push origin v0.18.0
```

The workflow runs the suite, builds four targets, and publishes a release with
the tarballs and `SHA256SUMS`. To build the artifacts **without** publishing,
run the workflow manually (`workflow_dispatch`); it stops after uploading the
artifacts.

## After it exists

1. **Verify what users will download**, from a different machine or a clean
   container:
   ```sh
   curl -fsSL https://raw.githubusercontent.com/erikvankempen/bukio-cli/main/install.sh | sh
   bukio --version
   ```
   Then confirm `SHA256SUMS` matches the published tarball by hand.
2. **Release notes.** `--generate-notes` writes the commit list; edit the release
   to lead with what changed for the user, the upgrade command, and the sentence
   that matters most: the database format is unchanged, so books and actor keys
   carry over and rolling back to 0.17 is safe.
3. **Announce**: the npm package (launcher or deprecation), the site, and
   whichever channels carry users.

## Details worth remembering

- **Platform floor.** The glibc artifacts are built on Ubuntu 22.04, so they need
  glibc 2.35+ — roughly "any distribution from 2022 on". Debian 11, RHEL 8 and
  Alpine need the **static musl** build; `install.sh` detects musl and asks for
  it. Enable the commented musl job in the workflow if you want it published; it
  needs `musl-tools` and `CC_<target>=musl-gcc` because bundled SQLite and `ring`
  both compile C.
- **No OpenSSL.** `lettre` is built with rustls and `rustls-native-certs`, so the
  binary links no libssl and an internal SMTP CA still verifies through the OS
  store. If someone reports a TLS problem on an unusual relay, this is the first
  thing to look at.
- **No toolchain for users.** The update path downloads an artifact and verifies
  its checksum. Do not reintroduce `cargo` or `git` into it — a user who
  installed a binary has neither, and that is the whole point of the Rust port.
- **macOS quarantine.** The installer clears the quarantine attribute; without
  that, Gatekeeper blocks the first run and agents get stuck. Notarisation is a
  later, paid step.
