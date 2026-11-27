# Releasing yo

The tag is the version. `Cargo.toml` keeps a placeholder (`0.0.0-dev`), and a
build from a checkout (`cargo build`, `cargo install --path`) says
`yo 0.0.0-dev`. A release is a `v<version>` tag on a commit whose CI passed:
on `main` for a normal release, on a `release/X.Y` branch for a hotfix. There
is no release PR, and nothing to bump.

Since any `v*` tag publishes, the repo's "release tags" ruleset lets only
repository admins create, move or delete `v*` tags. A released tag never
moves: its tarballs and the formula's checksums are tied to it.

Pushing the tag runs `.github/workflows/release.yml`, which calls
`cargo xtask release`:

1. It takes the version from the tag: `vX.Y.Z`, or `vX.Y.Z-<pre>` (for
   example `v0.2.0-rc.1`) for a pre-release. Anything else fails.
2. `cargo xtask dist` builds `yo` for `aarch64-apple-darwin` and
   `x86_64-apple-darwin` with `YO_VERSION` set to that version (yo-cli
   reads it at compile time), and packs each, with `LICENSE`, as
   `dist/yo-darwin_arm64.tar.gz` and `dist/yo-darwin_amd64.tar.gz`. The
   native one has to say `yo <version>` to `--version`, since the formula's
   test checks that.
3. It creates the GitHub Release with generated notes and the two tarballs.
   A pre-release is marked as one.
4. For a release (not a pre-release), it dispatches `update-formula` to
   `mmobeus/homebrew-tap` with the formula's name (`yo`), the tag and both
   checksums, and the tap's workflow commits the new `Formula/yo.rb`. A
   pre-release leaves the tap alone, so brew never sees it.

The workflow runs from the tagged commit, so a tag is released with the
`release.yml` and `xtask` that commit has. Tags from before this flow
(`v0.1.1` and earlier) check the tag against `Cargo.toml`; don't cut a
hotfix branch from one.

## Cutting a release

From an up-to-date `main` whose CI passed:

```sh
git tag v0.2.0 && git push origin v0.2.0
```

Then `brew upgrade yo` (or `brew install mmobeus/tap/yo`) picks it up once
the tap's workflow has run; `yo --version` says `yo 0.2.0`.

## A hotfix

Only when a fix has to ship without what's merged to `main` since the last
release. Land the fix on `main` first, then:

```sh
git switch -c release/0.2 v0.2.0       # or the branch, if an earlier hotfix made it
git cherry-pick <the fix's commit>
git push -u origin release/0.2         # CI runs on release/ branches too
# once CI passes:
git tag v0.2.1 && git push origin v0.2.1
```

A `release/X.Y` branch is made on demand and can be deleted once its tag is
pushed; the tag keeps the commit. Hotfix only the newest release line: the
tap follows whichever release went out last, so a `v0.1.x` after `v0.2.0`
would move brew back to 0.1.

## Trying it out

`cargo xtask dist` builds the tarballs without publishing anything, on
macOS. With `YO_VERSION=0.2.0` it builds them to say that version; without,
they say the placeholder.

To try the whole workflow without touching brew, push a pre-release tag
(`v0.2.1-rc.1`, from `main` or a `release/` branch), check the GitHub
Release, then delete it and the tag:

```sh
gh release delete v0.2.1-rc.1 --cleanup-tag --yes
```

## What it needs

| variable | what |
| --- | --- |
| `YO_RELEASE_TAG` | the tag being released; the workflow sets it from the pushed tag |
| `GH_TOKEN` | creates the release; the workflow's own token, with `contents: write` |
| `HOMEBREW_TAP_TOKEN` | can dispatch to `mmobeus/homebrew-tap`: a token from the org's Homebrew GitHub App (`HOMEBREW_APP_ID`, `HOMEBREW_APP_PRIVATE_KEY`, org secrets this repo has access to) |
| `YO_VERSION` | `cargo xtask dist` only, and optional: the version the build says (`release` sets it from the tag) |

`cargo xtask dist` needs none of the others. The binaries aren't notarized;
a formula's download isn't quarantined, so macOS doesn't need it to be.
