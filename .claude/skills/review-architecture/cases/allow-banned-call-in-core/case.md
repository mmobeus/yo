# allow-banned-call-in-core

- base: `6df999b893a1f355a1912e50d8dbfeba61646dc3`
- expect: flag, with a blocking finding citing `crates/yo-core/src/lib.rs`

`yo-core` runs `git` with `Command::new` outside `sys.rs`, behind an `#[allow(clippy::disallowed_methods)]` that gets it past the lint. That's a gate loosened, and an outside effect not going through `Sys`.
