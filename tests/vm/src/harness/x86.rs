//! 64-bit long-mode vCPU bring-up for the direct Linux boot protocol:
//! identity-mapped page tables (first 1 GiB, 2 MiB pages), a minimal GDT,
//! and the control registers. The standard recipe shared by the rust-vmm
//! VMMs; addresses live below the kernel's load address.

use std::io;

use kvm_ioctls::VcpuFd;
use vm_memory::{Bytes, GuestAddress, GuestMemoryMmap};

const GDT_ADDR: u64 = 0x500;
const PML4_ADDR: u64 = 0x9000;
const PDPT_ADDR: u64 = 0xA000;
const PD_ADDR: u64 = 0xB000;

const X86_CR0_PE: u64 = 1;
const X86_CR0_ET: u64 = 1 << 4;
const X86_CR0_PG: u64 = 1 << 31;
const X86_CR4_PAE: u64 = 1 << 5;
const EFER_LME: u64 = 1 << 8;
const EFER_LMA: u64 = 1 << 10;

fn gdt_entry(flags: u16, base: u32, limit: u32) -> u64 {
    ((base as u64 & 0xff00_0000) << 32)
        | ((flags as u64 & 0x0000_f0ff) << 40)
        | ((limit as u64 & 0x000f_0000) << 32)
        | ((base as u64 & 0x00ff_ffff) << 16)
        | (limit as u64 & 0x0000_ffff)
}

fn seg(selector: u16, flags: u16) -> kvm_bindings::kvm_segment {
    kvm_bindings::kvm_segment {
        base: 0,
        limit: 0xfffff,
        selector,
        type_: (flags & 0xf) as u8,
        present: 1,
        dpl: 0,
        db: ((flags >> 14) & 1) as u8,
        s: ((flags >> 4) & 1) as u8,
        l: ((flags >> 13) & 1) as u8,
        g: ((flags >> 15) & 1) as u8,
        avl: 0,
        padding: 0,
        unusable: 0,
    }
}

/// Write page tables + GDT into guest RAM and set sregs for long mode.
pub fn setup_long_mode(vcpu: &VcpuFd, mem: &GuestMemoryMmap) -> io::Result<()> {
    // Identity map 0..1 GiB with 2 MiB pages: PML4[0] -> PDPT[0] -> PD[512].
    mem.write_obj::<u64>(PDPT_ADDR | 0x3, GuestAddress(PML4_ADDR))
        .map_err(|e| io::Error::other(format!("pml4: {e}")))?;
    mem.write_obj::<u64>(PD_ADDR | 0x3, GuestAddress(PDPT_ADDR))
        .map_err(|e| io::Error::other(format!("pdpt: {e}")))?;
    for i in 0u64..512 {
        // present | rw | huge (PS)
        mem.write_obj::<u64>((i << 21) | 0x83, GuestAddress(PD_ADDR + i * 8))
            .map_err(|e| io::Error::other(format!("pd[{i}]: {e}")))?;
    }

    // GDT: null, 64-bit code (0x08), data (0x10).
    // code: type=0xb (exec/read accessed), s=1, l=1, g=1
    // data: type=0x3 (rw accessed), s=1, db=1, g=1
    let code_flags: u16 = 0xa09b; // g=1, l=1, s=1, present, type 0xb
    let data_flags: u16 = 0xc093; // g=1, db=1, s=1, present, type 0x3
    let gdt = [
        0u64,
        gdt_entry(code_flags, 0, 0xfffff),
        gdt_entry(data_flags, 0, 0xfffff),
    ];
    for (i, e) in gdt.iter().enumerate() {
        mem.write_obj::<u64>(*e, GuestAddress(GDT_ADDR + (i as u64) * 8))
            .map_err(|e| io::Error::other(format!("gdt: {e}")))?;
    }

    let mut sregs = vcpu.get_sregs().map_err(io::Error::from)?;
    sregs.gdt.base = GDT_ADDR;
    sregs.gdt.limit = (gdt.len() * 8 - 1) as u16;
    let code = seg(0x08, code_flags);
    let data = seg(0x10, data_flags);
    sregs.cs = code;
    sregs.ds = data;
    sregs.es = data;
    sregs.fs = data;
    sregs.gs = data;
    sregs.ss = data;
    sregs.cr3 = PML4_ADDR;
    sregs.cr4 |= X86_CR4_PAE;
    sregs.cr0 |= X86_CR0_PE | X86_CR0_ET | X86_CR0_PG;
    sregs.efer |= EFER_LME | EFER_LMA;
    vcpu.set_sregs(&sregs).map_err(io::Error::from)?;
    Ok(())
}
