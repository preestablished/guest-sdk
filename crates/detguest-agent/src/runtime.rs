//! PID 1 runtime: mounts, the boot sequence, boot faults, power-off
//! (ARCHITECTURE.md §4 steps 1–7, 11; API.md §7.3).
//!
//! Permitted-unsafe module: mount/reboot libc calls.
#![allow(unsafe_code)]

use std::{io, os::fd::AsRawFd};

use detguest_wire::events::{EventPayload, CAP_FORCED_QUIESCE, CAP_REVERIFY_REGIONS};
use detguest_wire::header::CHANNEL_SIZE_PAGES;
use detguest_wire::ports::{self, InitStatus};

use crate::boot::{self, BootManifest, ExpectedRegion};
use crate::channel::AgentChannel;
use crate::control;
use crate::supervise::{vnanos, Supervisor};
use crate::{agent_version, pio, translate};

/// Expected-regions gate spin cap. Each iteration services region IPC and
/// copies + decodes the manifest — tens of syscalls and a multi-KB copy,
/// order 10–20 K guest instructions — so like `CONTROL_RECV_POLL_LIMIT`
/// this is sized against the host's run budget: 100 K iterations ≈ 1–2 B
/// guest instructions, so a wedged gate boot-faults inside a worker's hard
/// cap instead of burning silently to it (request
/// phase3-ready-not-emitted-real-worker). Legitimate boots pass the gate in
/// a handful of polls — the regions register during the control leg that
/// precedes this wait.
#[cfg(test)]
const READY_REGION_POLL_LIMIT: usize = 128;
#[cfg(not(test))]
const READY_REGION_POLL_LIMIT: usize = 100_000;

/// Boot manifest path inside the initramfs (API.md §7).
pub const BOOT_TOML_PATH: &str = "/etc/detguest/boot.toml";

/// Print that can never panic and never depends on fds: best-effort
/// write(2) to stderr (ignored on failure) plus the emergency serial path.
pub fn console_log(msg: &str) {
    let line = format!("detguest-agent: {msg}\n");
    // SAFETY: plain write(2) to fd 2; failure is ignored by design.
    unsafe {
        libc::write(2, line.as_ptr() as *const libc::c_void, line.len());
    }
    crate::pio::emergency_serial(&line);
}

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

/// Step 2: mount /proc, /sys, devtmpfs, hugetlbfs — and wire stdio.
///
/// The initramfs carries no /dev/console node, so PID 1 starts with NO valid
/// fds 0–2: any println/eprintln before stdio setup would itself panic
/// (write to a closed fd), masking the real error with exit 101. So: mount
/// devtmpfs first, immediately bind fds 0–2 to /dev/console, and only then
/// do anything that can print.
pub fn mount_all() -> io::Result<()> {
    // devtmpfs before everything: stdio depends on it. EBUSY = already there.
    match mount("devtmpfs", "/dev", "devtmpfs") {
        Ok(()) => {}
        Err(e) if e.raw_os_error() == Some(libc::EBUSY) => {}
        Err(e) => {
            setup_stdio(); // best effort (/dev/null fallback)
            return Err(e);
        }
    }
    setup_stdio();
    mount("proc", "/proc", "proc")?;
    mount("sysfs", "/sys", "sysfs")?;
    std::fs::create_dir_all("/dev/hugepages")?;
    mount("hugetlbfs", "/dev/hugepages", "hugetlbfs")?;
    std::fs::create_dir_all("/run")?;
    Ok(())
}

/// Bind fds 0–2 to /dev/console (or /dev/null) so std's print macros have a
/// valid target. Failures here leave the fds as-is — nothing we can report.
fn setup_stdio() {
    // SAFETY: open + dup2 onto the standard fd numbers.
    unsafe {
        let console = std::ffi::CString::new("/dev/console").unwrap();
        let mut fd = libc::open(console.as_ptr(), libc::O_RDWR);
        if fd < 0 {
            let null = std::ffi::CString::new("/dev/null").unwrap();
            fd = libc::open(null.as_ptr(), libc::O_RDWR);
        }
        if fd >= 0 {
            for target in 0..3 {
                libc::dup2(fd, target);
            }
            if fd > 2 {
                libc::close(fd);
            }
        }
    }
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

/// Boot-leg breadcrumb: a tiny agent LogLine so a wedged boot names its
/// last completed leg in the host's buffered-event dump (request
/// phase3-ready-not-emitted-real-worker — the first real boot wedged
/// somewhere after region registration and died as a silent hard-cap).
/// Droppable emit: a full ring must not turn diagnostics into a new wedge.
fn breadcrumb(channel: &mut AgentChannel, msg: &str) {
    channel.emit(
        vnanos(),
        0,
        &EventPayload::LogLine {
            stream: detguest_wire::events::log_stream::AGENT,
            level: 1,
            msg: msg.as_bytes(),
        },
    );
}

/// Drive the boot-time control leg over `sock` and, on success, retain the
/// agent's end for the workload's lifetime. Dropping it closes the
/// workload's inherited fd 3, which its frame loop polls at every frame
/// boundary and treats EOF on as agent death — an early close is a
/// protocol violation (it killed the first real boot right after Ready:
/// request phase3-ready-not-emitted-real-worker, symptom 2).
fn drive_and_retain_control(
    sup: &mut Supervisor,
    sock: control::ControlSocket,
    unit_control: &crate::boot::UnitControl,
    game_path: &str,
) -> Result<(), String> {
    control::drive_refwork_start(&sock, unit_control, game_path, |p| match p {
        control::ControlProgress::Idle => sup
            .service_region_ipc()
            .map_err(|e| format!("region IPC service: {e}")),
        control::ControlProgress::Milestone(m) => {
            breadcrumb(&mut sup.channel, m);
            Ok(())
        }
    })?;
    sup.workload_control = Some(sock);
    Ok(())
}

/// Step 7: autostart + the READY gate (ARCHITECTURE.md §4.1).
///
/// With no autostart unit: `Ready` fires immediately after Hello with
/// `region_count = 0`. With one: start it agent-locally (no ring-C record),
/// then gate on every expected region being live at its pinned
/// layout_version.
pub fn autostart_and_ready(sup: &mut Supervisor) -> Result<(), String> {
    let unit = match sup.manifest.autostart_unit {
        None => {
            let snapshot = ready_manifest(sup)?;
            emit_ready(sup, 0xFFFF_FFFF, snapshot);
            return Ok(());
        }
        Some(u) => u,
    };
    let unit_control = sup
        .manifest
        .unit(unit)
        .and_then(|u| u.control.as_ref())
        .cloned();
    if let Some(control) = unit_control.as_ref() {
        // Resolve the LoadGame path BEFORE the unit is spawned, so a pv-blk
        // materialization fault never leaves an orphan workload running
        // (API.md §7.1: game_source = "pv-blk" ⇒ the agent reads the game
        // image out of the pv-blk device into a file the unit can read).
        let game_path: &str = match control.game_source {
            Some(boot::GameSource::PvBlk) => {
                crate::pvblk::materialize(crate::pvblk::GAME_IMG_PATH)
                    .map_err(|e| format!("materialize game from pv-blk: {e}"))?;
                crate::pvblk::GAME_IMG_PATH
            }
            // §7.2 guarantees game_dev for refwork-ctl; a non-refwork
            // protocol without one still faults in the protocol check.
            None => control
                .game_dev
                .as_deref()
                .ok_or_else(|| "refwork-ctl requires game_dev".to_string())?,
        };
        let (sock, child_fd) =
            control::socketpair().map_err(|e| format!("unit.control socketpair: {e}"))?;
        sup.start_unit_with_control(unit, child_fd.as_raw_fd())
            .map_err(|e| format!("autostart unit {unit}: {e}"))?;
        drop(child_fd);
        drive_and_retain_control(sup, sock, control, game_path)?;
        // The harness holds its own copy by GameLoaded; drop the RAM-backed
        // file so the game exists once at steady state.
        if control.game_source.is_some() {
            std::fs::remove_file(crate::pvblk::GAME_IMG_PATH)
                .map_err(|e| format!("unlink {}: {e}", crate::pvblk::GAME_IMG_PATH))?;
            breadcrumb(&mut sup.channel, "boot: game-unlinked");
        }
    } else {
        sup.start_unit(unit)
            .map_err(|e| format!("autostart unit {unit}: {e}"))?;
    }
    let expected_regions = sup.manifest.expected_regions.clone();
    let snapshot = wait_for_expected_regions(sup, &expected_regions)?;
    breadcrumb(&mut sup.channel, "boot: regions-gated");
    emit_expected_region_evidence(sup, &expected_regions, snapshot)?;
    breadcrumb(&mut sup.channel, "boot: evidence-done");
    emit_ready(sup, unit, snapshot);
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ReadyManifest {
    region_count: u32,
    manifest_generation: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RegionEvidence {
    region_id: u32,
    name_id: u32,
    name: Vec<u8>,
    layout_version: u32,
    manifest_generation: u32,
}

fn ready_manifest(sup: &Supervisor) -> Result<ReadyManifest, String> {
    let bytes = sup
        .channel
        .copy_manifest_stable()
        .map_err(|e| format!("read manifest before Ready: {e:?}"))?;
    let hdr = detguest_wire::manifest::ManifestHeader::read_from(&bytes)
        .map_err(|e| format!("decode manifest before Ready: {e:?}"))?;
    hdr.validate()
        .map_err(|e| format!("validate manifest before Ready: {e:?}"))?;
    Ok(ReadyManifest {
        region_count: hdr.region_count,
        manifest_generation: hdr.generation,
    })
}

fn expected_regions_ready(
    channel: &AgentChannel,
    expected_regions: &[ExpectedRegion],
) -> Result<ReadyManifest, String> {
    let (snapshot, _evidence) = expected_region_evidence(channel, expected_regions)?;
    Ok(snapshot)
}

fn expected_region_evidence(
    channel: &AgentChannel,
    expected_regions: &[ExpectedRegion],
) -> Result<(ReadyManifest, Vec<RegionEvidence>), String> {
    let bytes = channel
        .copy_manifest_stable()
        .map_err(|e| format!("read manifest before Ready: {e:?}"))?;
    let hdr = detguest_wire::manifest::ManifestHeader::read_from(&bytes)
        .map_err(|e| format!("decode manifest before Ready: {e:?}"))?;
    hdr.validate()
        .map_err(|e| format!("validate manifest before Ready: {e:?}"))?;
    let manifest_generation = u32::try_from(hdr.generation).map_err(|_| {
        format!(
            "manifest generation {} exceeds RegionRegister payload",
            hdr.generation
        )
    })?;
    let mut missing = Vec::new();
    let mut evidence = Vec::new();
    for expected in expected_regions {
        let mut found = None;
        for i in 0..hdr.region_count as usize {
            let entry = detguest_wire::manifest::RegionEntry::read_from(&bytes, i)
                .map_err(|e| format!("decode region manifest entry {i}: {e:?}"))?;
            if entry.is_live()
                && entry.layout_version == expected.layout_version
                && entry.name_bytes() == expected.name.as_bytes()
            {
                found = Some(RegionEvidence {
                    region_id: entry.region_id,
                    name_id: entry.name_id,
                    name: entry.name_bytes().to_vec(),
                    layout_version: entry.layout_version,
                    manifest_generation,
                });
                break;
            }
        }
        if let Some(found) = found {
            evidence.push(found);
        } else {
            missing.push(format!("{}@{}", expected.name, expected.layout_version));
        }
    }
    if !missing.is_empty() {
        return Err(format!(
            "expected_regions pending before Ready: {}",
            missing.join(", ")
        ));
    }
    let snapshot = ReadyManifest {
        region_count: hdr.region_count,
        manifest_generation: hdr.generation,
    };
    Ok((snapshot, evidence))
}

fn wait_for_expected_regions(
    sup: &mut Supervisor,
    expected_regions: &[ExpectedRegion],
) -> Result<ReadyManifest, String> {
    if expected_regions.is_empty() {
        // Even with nothing to gate on, drain any register requests that
        // raced the autostart so a fast workload isn't left blocked.
        sup.service_region_ipc()
            .map_err(|e| format!("region IPC service: {e}"))?;
        let bytes = sup
            .channel
            .copy_manifest_stable()
            .map_err(|e| format!("read manifest before Ready: {e:?}"))?;
        let hdr = detguest_wire::manifest::ManifestHeader::read_from(&bytes)
            .map_err(|e| format!("decode manifest before Ready: {e:?}"))?;
        hdr.validate()
            .map_err(|e| format!("validate manifest before Ready: {e:?}"))?;
        return Ok(ReadyManifest {
            region_count: hdr.region_count,
            manifest_generation: hdr.generation,
        });
    }
    let mut last_err = String::new();
    for _ in 0..READY_REGION_POLL_LIMIT {
        // Registrations arrive over agent.sock; the supervise epoll loop is
        // not running yet, so this wait IS the IPC service loop.
        sup.service_region_ipc()
            .map_err(|e| format!("region IPC service: {e}"))?;
        match expected_regions_ready(&sup.channel, expected_regions) {
            Ok(snapshot) => return Ok(snapshot),
            Err(err) => last_err = err,
        }
        // Give the just-forked workload a deterministic chance to publish.
        unsafe {
            libc::sched_yield();
        }
    }
    Err(format!(
        "expected-regions gate exhausted after {READY_REGION_POLL_LIMIT} polls: {last_err}"
    ))
}

fn emit_expected_region_evidence(
    sup: &mut Supervisor,
    expected_regions: &[ExpectedRegion],
    snapshot: ReadyManifest,
) -> Result<(), String> {
    if expected_regions.is_empty() {
        return Ok(());
    }
    let (fresh, evidence) = expected_region_evidence(&sup.channel, expected_regions)?;
    if fresh != snapshot {
        return Err("manifest changed while preparing Ready evidence".into());
    }
    for region in evidence {
        sup.channel.emit(
            vnanos(),
            0,
            &EventPayload::NameIntern {
                name_id: region.name_id,
                name: &region.name,
            },
        );
        sup.channel.emit(
            vnanos(),
            0,
            &EventPayload::RegionRegister(detguest_wire::events::RegionEvent {
                region_id: region.region_id,
                name_id: region.name_id,
                layout_version: region.layout_version,
                manifest_generation: region.manifest_generation,
            }),
        );
    }
    Ok(())
}

fn emit_ready(sup: &mut Supervisor, unit: u32, snapshot: ReadyManifest) {
    sup.channel.emit_with_doorbell(
        vnanos(),
        0,
        &EventPayload::Ready {
            unit,
            region_count: snapshot.region_count,
            manifest_generation: snapshot.manifest_generation,
        },
    );
}

/// The full PID 1 sequence. On any pre-Ready failure: boot fault (§7.3).
pub fn run() -> ! {
    if let Err(e) = mount_all() {
        // No channel yet — serial console is the only witness.
        console_log(&format!("mount failed: {e}"));
        power_off();
    }
    let mut channel = match bring_up_channel() {
        Ok(c) => c,
        Err(e) => {
            console_log(&format!("channel bring-up failed: {e}"));
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
            console_log(&format!("supervisor setup failed: {e}"));
            power_off();
        }
    };
    // agent.sock must exist before any workload runs (§5): a guest without
    // the region path must not reach Ready.
    match crate::region_ipc::RegionIpc::bind() {
        Ok(ipc) => sup.install_region_ipc(ipc),
        Err(e) => boot_fault(&mut sup.channel, &format!("bind agent.sock: {e}")),
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::boot::{ExpectedRegion, Unit, UnitControl};
    use detguest_wire::manifest::{writer_begin, writer_end, Extent, ManifestHeader, RegionEntry};
    use std::sync::atomic::{AtomicU32, Ordering};

    static DOORBELLS: AtomicU32 = AtomicU32::new(0);

    fn test_doorbell(_mask: u32) {
        DOORBELLS.fetch_add(1, Ordering::Relaxed);
    }

    fn manifest(
        control: Option<UnitControl>,
        expected_regions: Vec<ExpectedRegion>,
    ) -> BootManifest {
        BootManifest {
            autostart_unit: Some(0),
            units: vec![Unit {
                id: 0,
                exec: "/bin/true".to_string(),
                args: Vec::new(),
                log_mask: 0x1F,
                control,
            }],
            expected_regions,
        }
    }

    fn write_live_region(channel: &mut AgentChannel, name: &str, layout_version: u32) {
        let manifest = channel.manifest_mut();
        let odd = writer_begin(manifest).unwrap();
        core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);
        Extent {
            gpa: 0x2000,
            len: 16,
        }
        .write_to(manifest, 0)
        .unwrap();
        RegionEntry {
            region_id: 0,
            name_id: 1,
            layout_version,
            flags: 0,
            gva: 0x1000,
            len: 16,
            extent_off: 0,
            extent_n: 1,
            name: RegionEntry::pack_name(name.as_bytes()).unwrap(),
        }
        .write_to(manifest, 0)
        .unwrap();
        let mut hdr = ManifestHeader::read_from(manifest).unwrap();
        hdr.generation = odd;
        hdr.region_count = 1;
        hdr.extent_count = 1;
        hdr.write_to(manifest).unwrap();
        core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);
        writer_end(manifest).unwrap();
    }

    #[derive(Debug, PartialEq, Eq)]
    enum TestPayload {
        LogLine {
            stream: u8,
            msg: Vec<u8>,
        },
        NameIntern {
            name_id: u32,
            name: Vec<u8>,
        },
        RegionRegister {
            region_id: u32,
            name_id: u32,
            layout_version: u32,
            manifest_generation: u32,
        },
        Ready {
            unit: u32,
            region_count: u32,
            manifest_generation: u64,
        },
        Other(u32),
    }

    fn ring_a_payloads(channel: &AgentChannel) -> Vec<TestPayload> {
        let prod = unsafe {
            (channel
                .base_ptr()
                .add(detguest_wire::header::OFF_RING_A_PROD) as *const u32)
                .read_volatile()
        } as usize;
        let bytes = unsafe {
            std::slice::from_raw_parts(
                channel
                    .base_ptr()
                    .add(detguest_wire::header::OFF_RING_A_DATA),
                prod,
            )
        };
        let mut out = Vec::new();
        let mut at = 0;
        while at < bytes.len() {
            let (hdr, payload) = detguest_wire::events::decode_event(&bytes[at..]).unwrap();
            out.push(match payload {
                EventPayload::LogLine { stream, msg, .. } => TestPayload::LogLine {
                    stream,
                    msg: msg.to_vec(),
                },
                EventPayload::NameIntern { name_id, name } => TestPayload::NameIntern {
                    name_id,
                    name: name.to_vec(),
                },
                EventPayload::RegionRegister(region) => TestPayload::RegionRegister {
                    region_id: region.region_id,
                    name_id: region.name_id,
                    layout_version: region.layout_version,
                    manifest_generation: region.manifest_generation,
                },
                EventPayload::Ready {
                    unit,
                    region_count,
                    manifest_generation,
                } => TestPayload::Ready {
                    unit,
                    region_count,
                    manifest_generation,
                },
                _ => TestPayload::Other(hdr.kind as u32),
            });
            at += hdr.len as usize;
        }
        out
    }

    fn ring_a_has_ready(channel: &AgentChannel) -> bool {
        ring_a_payloads(channel)
            .iter()
            .any(|payload| matches!(payload, TestPayload::Ready { .. }))
    }

    #[test]
    fn expected_regions_pending_starts_unit_but_blocks_ready() {
        let mut sup = Supervisor::new(
            crate::channel::test_channel(test_doorbell),
            manifest(
                None,
                vec![ExpectedRegion {
                    name: "wram".to_string(),
                    layout_version: 1,
                }],
            ),
        )
        .unwrap();

        let err = autostart_and_ready(&mut sup).unwrap_err();

        assert!(err.contains("expected_regions pending before Ready"));
        assert!(sup.workload.is_some(), "autostart must happen before gate");
        assert!(!ring_a_has_ready(&sup.channel), "must not emit Ready");
    }

    #[test]
    fn expected_regions_ready_emit_real_manifest_snapshot() {
        let before = DOORBELLS.load(Ordering::Relaxed);
        let mut sup = Supervisor::new(
            crate::channel::test_channel(test_doorbell),
            BootManifest {
                autostart_unit: None,
                units: Vec::new(),
                expected_regions: Vec::new(),
            },
        )
        .unwrap();
        write_live_region(&mut sup.channel, "wram", 1);
        let expected = [ExpectedRegion {
            name: "wram".to_string(),
            layout_version: 1,
        }];

        let snapshot = expected_regions_ready(&sup.channel, &expected).unwrap();
        emit_expected_region_evidence(&mut sup, &expected, snapshot).unwrap();
        emit_ready(&mut sup, 0, snapshot);

        assert_eq!(snapshot.region_count, 1);
        assert_eq!(snapshot.manifest_generation, 2);
        assert!(
            DOORBELLS.load(Ordering::Relaxed) > before,
            "Ready must doorbell"
        );
        let payloads = ring_a_payloads(&sup.channel);
        assert_eq!(
            payloads,
            vec![
                TestPayload::NameIntern {
                    name_id: 1,
                    name: b"wram".to_vec(),
                },
                TestPayload::RegionRegister {
                    region_id: 0,
                    name_id: 1,
                    layout_version: 1,
                    manifest_generation: 2,
                },
                TestPayload::Ready {
                    unit: 0,
                    region_count: 1,
                    manifest_generation: 2,
                },
            ]
        );
    }

    fn send_dg(fd: &std::os::fd::OwnedFd, bytes: &[u8]) {
        use std::os::fd::AsRawFd;
        // SAFETY: `bytes` is readable for its full length and `fd` is live.
        let n = unsafe {
            libc::send(
                fd.as_raw_fd(),
                bytes.as_ptr().cast(),
                bytes.len(),
                libc::MSG_NOSIGNAL,
            )
        };
        assert_eq!(n, bytes.len() as isize, "{}", io::Error::last_os_error());
    }

    fn recv_dg(fd: &std::os::fd::OwnedFd) -> Vec<u8> {
        use std::os::fd::AsRawFd;
        let mut buf = [0u8; 4096];
        // SAFETY: blocking recv into a local buffer; `fd` is live.
        let n = unsafe { libc::recv(fd.as_raw_fd(), buf.as_mut_ptr().cast(), buf.len(), 0) };
        assert!(n > 0, "recv: {}", io::Error::last_os_error());
        buf[..n as usize].to_vec()
    }

    /// A scripted fd-3 peer speaking the harness's half of the boot leg
    /// (golden bytes from `control.rs` tests), returning its fd afterward
    /// so the test can probe whether the agent kept its own end open.
    fn fake_harness(
        child: std::os::fd::OwnedFd,
    ) -> std::thread::JoinHandle<std::os::fd::OwnedFd> {
        std::thread::spawn(move || {
            assert_eq!(recv_dg(&child), [0x00, 0x01], "Hello");
            send_dg(&child, &[0x05, 0x01, 0x03, b'e', b'm', b'u', 0x01, b'1']);
            assert_eq!(recv_dg(&child)[0], 0x01, "LoadGame");
            let mut game_loaded = vec![0x06];
            game_loaded.extend_from_slice(&[0u8; 32]);
            game_loaded.extend_from_slice(&[0x04, b'm', b'm', b'c', b'3', 0x00]);
            send_dg(&child, &game_loaded);
            send_dg(&child, &[0x08, 0x00]); // Ready { frame: 0 }
            assert_eq!(recv_dg(&child), [0x02], "Start");
            child
        })
    }

    /// Symptom-2 guard (request phase3-ready-not-emitted-real-worker): the
    /// agent must hold its end of the fd-3 socketpair for the workload's
    /// lifetime — the workload's frame loop treats EOF as agent death, and
    /// dropping the socket at the end of the boot leg killed the first real
    /// boot immediately after Ready. (Shown to fail when
    /// `drive_and_retain_control` is reverted to drop the socket instead of
    /// storing it — guard-reversion checked 2026-07-04.)
    #[test]
    fn control_leg_retains_workload_socket_and_names_its_legs() {
        use std::os::fd::AsRawFd;

        let mut sup =
            Supervisor::new(crate::channel::test_channel(test_doorbell), manifest(None, vec![]))
                .unwrap();
        let (sock, child) = control::socketpair().unwrap();
        let harness = fake_harness(child);
        let unit_control = UnitControl {
            protocol: "refwork-ctl".to_string(),
            proto_version: 1,
            game_dev: Some("/dev/vdb".to_string()),
            game_source: None,
        };

        drive_and_retain_control(&mut sup, sock, &unit_control, "/dev/vdb").unwrap();
        let child = harness.join().unwrap();

        assert!(
            sup.workload_control.is_some(),
            "boot-leg socket must be retained for the workload's lifetime"
        );
        // The workload-facing end must still be open with nothing queued:
        // EAGAIN, not the EOF (n == 0) that a dropped agent end produces.
        let mut buf = [0u8; 16];
        // SAFETY: non-blocking recv into a local buffer; `child` is live.
        let n = unsafe {
            libc::recv(
                child.as_raw_fd(),
                buf.as_mut_ptr().cast(),
                buf.len(),
                libc::MSG_DONTWAIT,
            )
        };
        let err = io::Error::last_os_error();
        assert_eq!(n, -1, "expected no data on an open socket, got n={n}");
        assert_eq!(err.kind(), io::ErrorKind::WouldBlock, "{err}");

        // The boot leg names each completed leg (wedge diagnosis
        // breadcrumbs — see the plan's step 01 decision table).
        let breadcrumbs: Vec<Vec<u8>> = ring_a_payloads(&sup.channel)
            .into_iter()
            .filter_map(|p| match p {
                TestPayload::LogLine { stream, msg }
                    if stream == detguest_wire::events::log_stream::AGENT =>
                {
                    Some(msg)
                }
                _ => None,
            })
            .collect();
        assert_eq!(
            breadcrumbs,
            vec![
                b"boot: helloack".to_vec(),
                b"boot: gameloaded".to_vec(),
                b"boot: rw-ready".to_vec(),
                b"boot: start-sent".to_vec(),
            ]
        );

        // Reap/teardown is the sanctioned close: only then may the
        // workload-facing end see EOF.
        drop(sup);
        // SAFETY: non-blocking recv into a local buffer; `child` is live.
        let n = unsafe {
            libc::recv(
                child.as_raw_fd(),
                buf.as_mut_ptr().cast(),
                buf.len(),
                libc::MSG_DONTWAIT,
            )
        };
        assert_eq!(n, 0, "supervisor teardown closes the retained socket");
    }

    #[test]
    fn unit_control_faults_before_ready_when_workload_does_not_reply() {
        let mut sup = Supervisor::new(
            crate::channel::test_channel(test_doorbell),
            manifest(
                Some(UnitControl {
                    protocol: "refwork-ctl".to_string(),
                    proto_version: 1,
                    game_dev: Some("/dev/vdb".to_string()),
                    game_source: None,
                }),
                Vec::new(),
            ),
        )
        .unwrap();

        let err = autostart_and_ready(&mut sup).unwrap_err();

        assert!(err.contains("recv refwork HelloAck"), "{err}");
        assert!(
            sup.workload.is_some(),
            "control fault happens after autostart"
        );
        assert!(!ring_a_has_ready(&sup.channel), "must not emit Ready");
    }
}
