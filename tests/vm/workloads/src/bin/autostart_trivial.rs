//! The trivial autostart workload for the M2 READY-point gate (bead kuu).
//!
//! Boot manifest: autostart unit with an EMPTY expected-regions list. The
//! agent forks+execs this before emitting `Ready`, so everything up to (and
//! after) the exec must be deterministic: no wall-clock reads, no entropy,
//! no filesystem scans (ARCHITECTURE.md §7).
//!
//! It deliberately never exits: M2 measures the `Ready` doorbell icount
//! across 10 consecutive boots with this unit running. The park loop blocks
//! on a futex with no timeout — a true park, with no periodic wakeups to
//! perturb icounts (`park` may return spuriously; the loop re-parks).
#![forbid(unsafe_code)]

use std::io::Write;
use std::thread;

fn main() {
    // One deterministic mark on stdout (the agent relays it as a LogLine on
    // ring A, stream 1), then park forever. Ignore write errors — losing the
    // mark must not change control flow (§7 rule 9: deterministic fallibility).
    let _ = writeln!(std::io::stdout(), "autostart-trivial: up");
    let _ = std::io::stdout().flush();
    loop {
        thread::park();
    }
}
