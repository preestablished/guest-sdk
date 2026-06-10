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
            sup.start_unit(unit)?;
            if log_mask != 0 {
                sup.log_mask = log_mask;
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
            // M2 functional stub (bead 6c0): no SDK regions exist yet, so a
            // pagemap re-walk has nothing to verify; emit nothing. The full
            // re-walk + RegionUpdate emission lands with the M3 registration
            // path. Receiving the command must not fault — and does not.
        }
    }
    Ok(false)
}
