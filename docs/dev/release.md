# Releasing yo

A release is a `v<version>` tag on `main`. Pushing it runs
`.github/workflows/release.yml`, which calls `cargo xtask release`:

1. It checks that the tag is `v` plus the workspace version in `Cargo.toml`.
2. `cargo xtask dist` builds `yo` for `aarch64-apple-darwin` and
   `x86_64-apple-darwin`, and packs each as `dist/yo-darwin_arm64.tar.gz` and
   `dist/yo-darwin_amd64.tar.gz`. The native one has to say `yo <version>`
   to `--version`, since the formula's test checks that.
3. It creates the GitHub Release with generated notes and the two tarballs.
4. It dispatches `update-formula` to `mmobeus/homebrew-tap` with the
   formula's name (`yo`), the tag and both checksums, and the tap's workflow
   commits the new `Formula/yo.rb`.

## Cutting one

```sh
# on a branch: bump `version` in the root Cargo.toml (workspace.package)
cargo build                        # updates Cargo.lock
cargo xtask ci && cargo xtask dist # dist needs macOS
# PR, merge, then from an up-to-date main:
git tag v0.2.0 && git push origin v0.2.0
```

Then `brew upgrade yo` (or `brew install mmobeus/tap/yo`) picks it up once
the tap's workflow has run.

## What it needs

| variable | what |
| --- | --- |
| `YO_RELEASE_TAG` | the tag being released; the workflow sets it from the pushed tag |
| `GH_TOKEN` | creates the release; the workflow's own token, with `contents: write` |
| `HOMEBREW_TAP_TOKEN` | can dispatch to `mmobeus/homebrew-tap`: a token from the org's Homebrew GitHub App (`HOMEBREW_APP_ID`, `HOMEBREW_APP_PRIVATE_KEY`, org secrets this repo has access to) |

`cargo xtask dist` needs none of them, so it's how to check a release build
locally. The binaries aren't notarized; a formula's download isn't
quarantined, so macOS doesn't need it to be.
