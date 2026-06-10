//! The stdout/stderr printing workload for M2 LogLine/WorkloadExited
//! verification (bead 8i1).
//!
//! Emits a fixed, deterministic line pattern on both streams and exits with
//! a fixed nonzero code, so the harness can assert:
//! - `LogLine` events with stream 1 (stdout) carrying exactly these stdout
//!   lines, and stream 2 (stderr) carrying exactly these stderr lines
//!   (framing per API.md §3.2);
//! - `WorkloadExited { exit_code: 7, term_signal: 0 }` (critical) after the
//!   agent reaps it.
//!
//! Interleaving across the two pipes is scheduler-visible, so the harness
//! asserts per-stream sequences, not a global order. Write errors are
//! ignored rather than panicking (a closed pipe must yield exit code 7,
//! never Rust's panic exit 101 — the exit code is the assertion target).
#![forbid(unsafe_code)]

use std::io::Write;

const EXIT_CODE: i32 = 7;

fn main() {
    let mut out = std::io::stdout();
    let mut err = std::io::stderr();
    for i in 1..=5 {
        let _ = writeln!(out, "print-lines stdout {i}");
    }
    let _ = out.flush();
    for i in 1..=3 {
        let _ = writeln!(err, "print-lines stderr {i}");
    }
    std::process::exit(EXIT_CODE);
}
