# Positive Notes

- **Root-cause fix, not a symptom patch.** The change addresses *why* the
  musl default appears (cargo-fuzz's compiled-in host triple) by explicitly
  decoupling the fuzz build target from the binary's linkage, rather than
  hacking around the std-missing error.

- **Excellent inline documentation.** The three-line comment names the cause
  (prebuilt cargo-fuzz is musl-linked), the effect (defaults to musl, no std
  for sanitizer build), and the provenance (found on the first dispatch).
  A future maintainer will understand the `--target` immediately and not be
  tempted to "simplify" it away.

- **Verified reasoning.** The explanation matches cargo-fuzz 0.13.2 source
  exactly: `default_target()` returns `current_platform::CURRENT_PLATFORM`,
  a build-time constant of the cargo-fuzz binary's own host. The diff's
  mental model is precisely correct.

- **Minimal blast radius.** One logical change, one file, one commit. The
  elapsed-time gate, artifact upload, and all other steps are untouched.

- **YAML and build both still green locally.** Parses cleanly; the fuzz
  target builds with exit 0 under the new explicit triple.

- **Resilient to upstream drift.** Because the target is now explicit, the
  job no longer silently depends on how taiki-e links its prebuilt
  cargo-fuzz — if they switch linkage again, this job keeps working.
