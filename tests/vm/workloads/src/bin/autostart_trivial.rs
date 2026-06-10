//! The trivial autostart workload for the M2 READY-point gate (bead kuu).
//!
//! Boot manifest: autostart unit with an EMPTY expected-regions list. The
//! agent forks+execs this before emitting `Ready`, so everything up to (and
//! after) the exec must be deterministic: no wall-clock reads, no entropy,
//! no filesystem scans — just a virtual-time sleep loop the hypervisor's
//! timer virtualization fully controls (ARCHITECTURE.md §7).
//!
//! It deliberately never exits: M2 measures the `Ready` doorbell icount
//! across 10 consecutive boots with this unit running.

use std::thread;
use std::time::Duration;

fn main() {
    // One deterministic mark on stdout (the agent relays it as a LogLine on
    // ring A, stream 1), then park forever in virtual time.
    println!("autostart-trivial: up");
    loop {
        thread::sleep(Duration::from_secs(3600));
    }
}
