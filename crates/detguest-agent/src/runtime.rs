//! PID 1 runtime: mounts, the boot sequence, boot faults, power-off
//! (ARCHITECTURE.md §4 steps 1–7, 11; API.md §7.3).
//!
//! Permitted-unsafe module: mount/reboot libc calls.
#![allow(unsafe_code)]

use std::io;

use detguest_wire::events::{EventPayload, CAP_FORCED_QUIESCE, CAP_REVERIFY_REGIONS};
use detguest_wire::header::CHANNEL_SIZE_PAGES;
use detguest_wire::ports::{self, InitStatus};

use crate::boot::{self, BootManifest};
use crate::channel::AgentChannel;
use crate::supervise::{vnanos, Supervisor};
use crate::{agent_version, pio, translate};

/// Boot manifest path inside the initramfs (API.md §7).
pub const BOOT_TOML_PATH: &str = "/etc/detguest/boot.toml";

fn mount(src: &str, target: &str, fstype: &str) -> io::Result<()> {
    let s = std::ffi::CString::new(src).unwrap();
    let t = std::ffi::CString::new(target).unwrap();
    let f = std::ffi::CString::new(fstype).unwrap();
    // SAFETY: plain mount(2); all strings are NUL-terminated CStrings.
    let rc = unsafe { libc::mount(s.as_ptr(), t.as_ptr(), f.as_ptr(), 0, std::ptr::null()) };
    if rc != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// Step 2: mount /proc, /sys, devtmpfs, hugetlbfs.
pub fn mount_all() -> io::Result<()> {
    mount("proc", "/proc", "proc")?;
    mount("sysfs", "/sys", "sysfs")?;
    // devtmpfs may already be auto-mounted; EBUSY is fine.
    match mount("devtmpfs", "/dev", "devtmpfs") {
        Ok(()) => {}
        Err(e) if e.raw_os_error() == Some(libc::EBUSY) => {}
        Err(e) => return Err(e),
    }
    std::fs::create_dir_all("/dev/hugepages")?;
    mount("hugetlbfs", "/dev/hugepages", "hugetlbfs")?;
    std::fs::create_dir_all("/run")?;
    Ok(())
}

/// Power off the VM (step 11 / §7.3). Refuses to act unless we are PID 1 —
/// running the agent on a development host must never reboot it.
pub fn power_off() -> ! {
    // SAFETY: sync has no preconditions.
    unsafe { libc::sync() };
    if std::process::id() == 1 {
        // SAFETY: PID 1 in the guest; this is the spec'd power-off path.
        unsafe { libc::reboot(libc::LINUX_REBOOT_CMD_POWER_OFF) };
    }
    // reboot only returns on error (or non-PID1); exit loud either way.
    std::process::exit(1);
}

/// The §7.3 boot fault path: emit the detail as an agent LogLine (stream 3,
/// level 0), never emit Ready, power off.
pub fn boot_fault(channel: &mut AgentChannel, detail: &str) -> ! {
    channel.emit_with_doorbell(
        vnanos(),
        0,
        &EventPayload::LogLine {
            stream: detguest_wire::events::log_stream::AGENT,
            level: 0,
            msg: detail.as_bytes(),
        },
    );
    power_off()
}

/// Steps 3–6: allocate the channel, resolve its GPA, CHANNEL_INIT, Hello.
pub fn bring_up_channel() -> io::Result<AgentChannel> {
    let mut channel = AgentChannel::alloc(pio::doorbell)?;
    pio::raise_iopl()?;
    if !pio::ident_ok() {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "IDENT detcall mismatch: not running under the determinism hypervisor",
        ));
    }
    let pagemap = translate::open_pagemap()?;
    let gpa = translate::gva_to_gpa(&pagemap, channel.base_ptr() as u64)
        .map_err(|e| io::Error::other(format!("channel GPA translation: {e:?}")))?;
    let status = pio::channel_init(gpa, CHANNEL_SIZE_PAGES);
    if InitStatus::from_u32(status) != Some(InitStatus::Ok) {
        return Err(io::Error::other(format!("CHANNEL_INIT status {status}")));
    }
    channel.set_agent_ready();
    channel.emit_with_doorbell(
        vnanos(),
        0,
        &EventPayload::Hello {
            proto_version: detguest_wire::PROTO_VERSION,
            agent_version: agent_version(),
            capabilities: CAP_FORCED_QUIESCE | CAP_REVERIFY_REGIONS,
        },
    );
    Ok(channel)
}

/// Step 7: autostart + the READY gate (ARCHITECTURE.md §4.1).
///
/// With no autostart unit: `Ready` fires immediately after Hello with
/// `region_count = 0`. With one: start it agent-locally (no ring-C record),
/// then gate on every expected region being live at its pinned
/// layout_version. v1 M2 ships the empty-expected-regions path; a non-empty
/// list cannot be satisfied before the M3 registration path exists, so it
/// faults loudly rather than hanging the boot.
pub fn autostart_and_ready(sup: &mut Supervisor) -> Result<(), String> {
    let unit = match sup.manifest.autostart_unit {
        None => {
            emit_ready(sup, 0xFFFF_FFFF);
            return Ok(());
        }
        Some(u) => u,
    };
    if !sup.manifest.expected_regions.is_empty() {
        // M3: wait for manifest liveness (+ layout_version match) via the
        // registration path; M2 images must use an empty list.
        return Err(format!(
            "expected_regions not satisfiable before the M3 registration path \
             ({} regions listed)",
            sup.manifest.expected_regions.len()
        ));
    }
    if sup
        .manifest
        .unit(unit)
        .and_then(|u| u.control.as_ref())
        .is_some()
    {
        // §4.2 control-protocol leg is M4 work (reference-workload side).
        return Err("unit.control protocol leg not implemented before M4".into());
    }
    sup.start_unit(unit)
        .map_err(|e| format!("autostart unit {unit}: {e}"))?;
    emit_ready(sup, unit);
    Ok(())
}

fn emit_ready(sup: &mut Supervisor, unit: u32) {
    // Manifest generation is 0 (even) until the first registration; the
    // region count is the number of live manifest entries (0 in M2).
    sup.channel.emit_with_doorbell(
        vnanos(),
        0,
        &EventPayload::Ready {
            unit,
            region_count: 0,
            manifest_generation: 0,
        },
    );
}

/// The full PID 1 sequence. On any pre-Ready failure: boot fault (§7.3).
pub fn run() -> ! {
    if let Err(e) = mount_all() {
        // No channel yet — serial console is the only witness.
        eprintln!("detguest-agent: mount failed: {e}");
        power_off();
    }
    let mut channel = match bring_up_channel() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("detguest-agent: channel bring-up failed: {e}");
            power_off();
        }
    };
    let manifest: BootManifest = match std::fs::read_to_string(BOOT_TOML_PATH) {
        Ok(text) => match boot::parse(&text) {
            Ok(m) => m,
            Err(e) => boot_fault(&mut channel, &e.to_string()),
        },
        Err(e) => boot_fault(&mut channel, &format!("read {BOOT_TOML_PATH}: {e}")),
    };
    let mut sup = match Supervisor::new(channel, manifest) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("detguest-agent: supervisor setup failed: {e}");
            power_off();
        }
    };
    if let Err(detail) = autostart_and_ready(&mut sup) {
        boot_fault(&mut sup.channel, &detail);
    }
    match sup.run() {
        Ok(()) => power_off(),
        Err(e) => {
            // Post-Ready supervise failure: report and halt loudly. Doorbell
            // first so a full ring is drained and the (droppable) LogLine
            // has space to land.
            crate::pio::doorbell(detguest_wire::ports::DOORBELL_RING_A);
            sup.channel.emit_with_doorbell(
                vnanos(),
                0,
                &EventPayload::LogLine {
                    stream: detguest_wire::events::log_stream::AGENT,
                    level: 0,
                    msg: format!("supervise loop failed: {e}").as_bytes(),
                },
            );
            power_off();
        }
    }
}

// Keep the unused-import lint honest for ports (used in doc paths).
const _: u16 = ports::PORT_IDENT;
