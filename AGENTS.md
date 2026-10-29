# AGENTS.md

## Releasing

Follow [RELEASING.md](RELEASING.md): open a `chore(release): X.Y.Z` PR that gives the version
its `CHANGELOG.md` section, and merging it tags the commit and runs `.github/workflows/release.yml`,
which publishes to crates.io. Don't run `cargo publish` or push the tag by hand; a manual publish
makes the workflow's publish step fail.

Before opening the PR, validate locally:
- `cargo test`
- `cargo test --manifest-path utf16-simd/Cargo.toml`
- `cargo publish --dry-run`

After the merge, verify:
- `gh run list --workflow release.yml --limit 5`
- `gh release view vX.Y.Z`
- crates.io shows the new versions.
