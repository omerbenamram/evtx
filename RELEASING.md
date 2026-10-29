# Releasing

`evtx` (and `utf16-simd`, when its version is new) is published to crates.io, with `evtx_dump`
binaries attached to a GitHub release.

## One-time setup

- A crates.io token in the `CRATES_IO_TOKEN` Actions secret.

## Cut a release

1. Pick the version. While on 0.x, a breaking change or a new feature bumps the minor version
   and fixes alone bump the patch version. `cargo semver-checks check-release` flags API breaks.
2. Open a `chore(release): X.Y.Z` PR that bumps `version` in `Cargo.toml`, refreshes
   `Cargo.lock`, and renames `## [Unreleased]` in `CHANGELOG.md` to `## [X.Y.Z - YYYY-MM-DD]`.
   If `utf16-simd` changed since its last release, bump its version too.
3. Merge it.

The merge runs `.github/workflows/auto-tag.yml`. It sees a `Cargo.toml` version with no tag,
pushes the `vX.Y.Z` tag on the merge commit, and calls `.github/workflows/release.yml`:
GitHub release, binaries, then `cargo publish` last.

Pushing a `vX.Y.Z` tag by hand, or running `release.yml` from the Actions tab, still works;
auto-tag skips a version whose tag exists. To retry a failed release, re-run the failed jobs of
its run.

The Python bindings ([pyevtx-rs](https://github.com/omerbenamram/pyevtx-rs)) release next,
once the crate is on crates.io.
