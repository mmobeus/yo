# Running the gates

Every check CI runs is a `cargo xtask` gate, and runs the same way on your
machine. CI only checks out the code, installs the toolchain and the tools
below, and calls `cargo xtask <gate>`.

```
cargo xtask ci          # every default gate, in CI's order
cargo xtask <gate>      # one gate: lint, arch, deny, test, parity
cargo xtask advisories  # needs the network
cargo xtask help
```

The default gates need no secrets, and nothing from the network beyond
fetching crates, so `cargo xtask ci` works on a fresh clone. They run on Linux
and macOS; CI uses Linux.

## Tools

The toolchain is pinned in `rust-toolchain.toml`, and `rustup` installs it on
first use. The gates also need:

| tool | install |
| --- | --- |
| `cargo-nextest` | `cargo install --locked cargo-nextest` |
| `cargo-deny` | `cargo install --locked cargo-deny` |

A gate that needs a missing tool says so and how to install it.

## Environment

Anything that differs between a laptop and CI is an environment variable, and
every one `xtask` or the gates read is listed here. CI sets them in the
workflow, or from its secrets; locally they come from your shell, or from a
gitignored `.env` in the repo root (`KEY=VALUE` lines; your shell's value
wins). `xtask` passes them to every command a gate runs, and never asks
whether it's in CI.

| variable | used by | meaning |
| --- | --- | --- |
| _none yet_ | | |

Cargo's own variables (`CARGO_TERM_COLOR`, `CARGO_TARGET_DIR`, ...) work as
usual, and can go in `.env` too.
