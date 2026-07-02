//! Ring-C command dispatch (API.md §6; ARCHITECTURE.md §4 step 8).
//!
//! The poll itself lives on [`crate::channel::AgentChannel::poll_command`];
//! this module maps each decoded [`Command`] onto supervisor actions.

use std::io;

use detguest_wire::events::{Command, QuiesceMode, ShutdownMode};

use crate::supervise::Supervisor;

/// Handle one ring-C command. Returns `Ok(true)` when the supervise loop
/// must stop now (immediate shutdown); graceful shutdown returns `Ok(false)`
/// and completes via the loop's deadline sweep.
pub fn handle(sup: &mut Supervisor, cmd: Command) -> io::Result<bool> {
    match cmd {
        Command::StartWorkload { unit, log_mask } => {
            // `unit` selects among the boot manifest's preconfigured entries;
            // argv is NEVER sent over the wire (ARCHITECTURE.md §4 step 9).
            match sup.start_unit(unit) {
                Ok(()) => {
                    // API.md §6: apply the command's log_mask. 0 is a legal
                    // mask meaning "silence all levels".
                    sup.log_mask = log_mask;
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    // Refused; report host-ward, keep supervising.
                    let msg = format!("StartWorkload {unit} refused: {e}");
                    let v = crate::supervise::vnanos();
                    sup.channel.emit(
                        v,
                        0,
                        &detguest_wire::events::EventPayload::LogLine {
                            stream: detguest_wire::events::log_stream::AGENT,
                            level: 0,
                            msg: msg.as_bytes(),
                        },
                    );
                }
                Err(e) => return Err(e),
            }
        }
        Command::Quiesce {
            token,
            mode: QuiesceMode::Coop,
        } => {
            // Relay onto ring I; the SDK parks at its next quiesce_check()
            // and emits QuiesceReady itself (§6 cooperative path).
            sup.relay_quiesce(token);
        }
        Command::Quiesce {
            token,
            mode: QuiesceMode::Forced,
        } => {
            sup.forced_quiesce(token);
        }
        Command::Resume { token } => {
            // Ring-C Resume is the FORCED path (§6); also relay onto ring I
            // so a cooperatively-parked SDK (stale token case) unparks.
            sup.forced_resume();
            sup.relay_resume(token);
        }
        Command::Shutdown {
            mode: ShutdownMode::Graceful,
        } => {
            sup.begin_graceful_shutdown();
        }
        Command::Shutdown {
            mode: ShutdownMode::Immediate,
        } => {
            sup.immediate_shutdown();
            return Ok(true);
        }
        Command::SetLogMask { mask } => {
            sup.log_mask = mask;
        }
        Command::ReverifyRegions => {
            // API.md §6: re-walk pagemap for every live region; RegionUpdate
            // echo when extents hold, P0 alarm + rewrite/DEAD when they
            // don't. The ledger lives on the region IPC server (§5).
            sup.reverify_regions();
        }
    }
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::boot::BootManifest;
    use std::sync::atomic::{AtomicU32, Ordering};

    static DOORBELLS: AtomicU32 = AtomicU32::new(0);

    fn test_doorbell(_mask: u32) {
        DOORBELLS.fetch_add(1, Ordering::Relaxed);
    }

    #[test]
    fn reverify_regions_with_no_ledger_emits_nothing() {
        let before = DOORBELLS.load(Ordering::Relaxed);
        let mut sup = crate::supervise::Supervisor::new(
            crate::channel::test_channel(test_doorbell),
            BootManifest::default(),
        )
        .unwrap();

        let stop_now = handle(&mut sup, Command::ReverifyRegions).unwrap();

        assert!(!stop_now);
        assert!(sup.workload.is_none());
        assert!(sup.shutdown.is_none());
        assert_eq!(
            DOORBELLS.load(Ordering::Relaxed),
            before,
            "must not emit RegionUpdate or any other doorbell"
        );
    }
}
