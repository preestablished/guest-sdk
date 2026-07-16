# Critical & Important Findings

## CRITICAL — C1: Missing `rust-src` component will fail the next run (build-std)

**File:** `.github/workflows/fuzz.yaml:27` (toolchain step) and `:43-44` (run)

The target-triple fix unblocks target resolution but immediately runs into
a second, deterministic failure. `cargo fuzz run` defaults
`-Zbuild-std=true` (confirmed in `cargo fuzz run --help`: build-std
"defaults to true"). Building std from source **requires the `rust-src`
rustup component**.

`dtolnay/rust-toolchain@nightly` installs with `profile = minimal` by
default (rustc + rust-std + cargo only). It does **not** install
`rust-src`. So the next dispatch run will fail with:

```
error: ".../nightly-x86_64-unknown-linux-gnu/lib/rustlib/src/rust/library/Cargo.lock"
does not exist, unable to build with the standard library, try:
        rustup component add rust-src --toolchain nightly-x86_64-unknown-linux-gnu
```

I reproduced this exact failure locally by removing `rust-src` and running
`cargo +nightly build -Z build-std=std --target x86_64-unknown-linux-gnu`.

**Why the first dispatch didn't surface it:** the run died earlier at
target resolution (E0463 on the musl target). With the target now pinned to
gnu, the build proceeds far enough to attempt build-std, where rust-src
becomes mandatory. This is the next domino, not a hypothetical.

**Fix:** add the component to the toolchain step:

```yaml
- uses: dtolnay/rust-toolchain@nightly
  with:
    components: rust-src
```

(Alternatively, the locally-installed dev environment masks this because
`rust-src` happens to be installed there — so it cannot be caught without
running in CI or a minimal-profile environment.)

This single line is the difference between this PR fixing the workflow vs.
trading one red run for another.

## IMPORTANT — I1: Rationale comment slightly overstates the mechanism

**File:** `.github/workflows/fuzz.yaml:38-40`

The comment says the musl cargo-fuzz "would otherwise default to the musl
target, which has no std for the sanitizer build." That is accurate, but
the *mechanism* worth recording is that cargo-fuzz's `--target` default is
**its own build-time host triple** (`env!`-baked `DEFAULT_TARGET`), not the
runtime `rustc` host. Because `taiki-e/install-action` fetches the prebuilt
musl release, the baked default is musl. Without that detail, a future
maintainer who tests locally (where cargo-fuzz is gnu-linked and the
default is already gnu) cannot reproduce the failure and may "simplify" the
`--target` flag away as redundant. A one-line note that the default tracks
the *installed binary's* host would harden the fix against regression.

This is a comment-quality nit, not a functional defect — hence Important,
not Critical. But given the whole PR is one line plus a comment, the
comment carrying the full reason matters more than usual.
