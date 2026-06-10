//! detcall PIO layer: `iopl(3)` + 32-bit OUT/IN wrappers (ARCHITECTURE.md §2).
//!
//! Permitted-unsafe module: port I/O is inline asm by nature. The detcall
//! ports (0xD370–0xD39F) sit above `ioperm(2)`'s 0–0x3FF limit, so the agent
//! raises the I/O privilege level via `iopl(3)` (root with CAP_SYS_RAWIO in
//! the minimal image; the all-ports grant is security-irrelevant in a
//! trusted lab guest).
#![allow(unsafe_code)]

use detguest_wire::ports;

/// Raise IOPL to 3. Must be called once before any detcall.
pub fn raise_iopl() -> std::io::Result<()> {
    // SAFETY: iopl(3) has no memory effects; it changes the task's I/O
    // privilege. Requires CAP_SYS_RAWIO (we are root PID 1).
    let rc = unsafe { libc::iopl(3) };
    if rc != 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

/// 32-bit port write (`OUT port, eax`). Every detcall OUT is a synchronous
/// VM exit handled with the vCPU paused (deterministic by construction).
///
/// Deliberately NOT `options(nomem)`: the host drains channel memory inside
/// the exit, so the OUT must behave as a memory barrier to the compiler —
/// the normative "record visible before the OUT" discipline
/// (ARCHITECTURE.md §2, API.md §5) would otherwise allow the compiler to
/// hoist the OUT above the `Release` store publishing the record.
#[inline]
pub fn out32(port: u16, value: u32) {
    // Belt-and-braces: an explicit compiler fence in addition to the asm
    // block's default memory clobber.
    core::sync::atomic::compiler_fence(core::sync::atomic::Ordering::SeqCst);
    // SAFETY: requires IOPL≥3 (raise_iopl); the hypervisor handles the exit.
    unsafe {
        core::arch::asm!("out dx, eax", in("dx") port, in("eax") value, options(nostack));
    }
}

/// 32-bit port read (`IN eax, port`). The returned value is recorded by the
/// host in the input log (replay re-answers it identically). Not `nomem`
/// for the same ordering reason as [`out32`] (the host may have mutated
/// channel memory inside the exit).
#[inline]
pub fn in32(port: u16) -> u32 {
    let value: u32;
    // SAFETY: requires IOPL≥3.
    unsafe {
        core::arch::asm!("in eax, dx", in("dx") port, out("eax") value, options(nostack));
    }
    core::sync::atomic::compiler_fence(core::sync::atomic::Ordering::SeqCst);
    value
}

/// `IN 0xD370` identify; checks the magic+proto answer (API.md §5).
pub fn ident_ok() -> bool {
    in32(ports::PORT_IDENT) == ports::IDENT_VALUE
}

/// CHANNEL_INIT sequence (ARCHITECTURE.md §4 step 5): latch the channel GPA,
/// commit with the size in 4 KiB pages, read back the status.
pub fn channel_init(gpa: u64, size_pages: u32) -> u32 {
    out32(ports::PORT_INIT_LO, gpa as u32);
    out32(ports::PORT_INIT_HI, (gpa >> 32) as u32);
    out32(ports::PORT_INIT_GO, size_pages);
    in32(ports::PORT_INIT_GO)
}

/// Ring the doorbell for the rings in `mask` (bit0 = A, bit1 = W).
pub fn doorbell(mask: u32) {
    out32(ports::PORT_DOORBELL, mask);
}

/// 8-bit port write (`OUT port, al`) — the emergency serial path.
#[inline]
pub fn out8(port: u16, value: u8) {
    core::sync::atomic::compiler_fence(core::sync::atomic::Ordering::SeqCst);
    // SAFETY: requires IOPL>=3; no Rust-visible memory effects.
    unsafe {
        core::arch::asm!("out dx, al", in("dx") port, in("al") value, options(nostack));
    }
}

/// Last-resort boot diagnostics: bytes straight to the 8250 THR (0x3F8).
/// Needs no filesystem, no fds — only IOPL, which this raises idempotently.
/// Errors here are unreportable by definition; ignore them.
pub fn emergency_serial(msg: &str) {
    // Without IOPL an OUT is a GPF — worse than losing the message.
    if raise_iopl().is_err() {
        return;
    }
    for b in msg.bytes() {
        out8(0x3F8, b);
    }
    out8(0x3F8, b'\n');
}
