//! Seqlock-consistent manifest reads + `read_region` extent walks (API.md §2, §4).

use detguest_wire::header::OFF_MANIFEST;
use detguest_wire::manifest::{
    Extent, ManifestHeader, RegionEntry, EXTENT_CAPACITY, MANIFEST_TOTAL_SIZE, OFF_GENERATION,
    REGION_CAPACITY,
};

use crate::channel::Channel;
use crate::guestmem::{GuestMem, GuestMemExt};
use crate::{RegionReadError, WireError};

/// Bounded seqlock retries. The host reads while the vCPU is paused in
/// practice, so a retry only happens when the pause landed mid-registration;
/// exceeding this bound means the generation word is corrupt or stuck odd.
const SEQLOCK_RETRIES: usize = 64;

/// A consistent snapshot of the region manifest.
#[derive(Debug, Clone)]
pub struct RegionManifest {
    /// Parsed, validated manifest header.
    pub header: ManifestHeader,
    /// All 64 entry slots (dead entries keep their slots; check
    /// [`RegionEntry::is_live`]).
    pub entries: Vec<RegionEntry>,
    /// The used prefix of the extent pool (`header.extent_count` slots).
    pub extents: Vec<Extent>,
}

/// One resolved live region: its manifest fields plus its extent list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedRegion {
    /// Manifest slot index.
    pub region_id: u32,
    /// Workload-declared layout version (feature maps bind to it).
    pub layout_version: u32,
    /// Region length in bytes.
    pub len: u64,
    /// RegionFlags bits.
    pub flags: u32,
    /// Extents in logical concatenation order.
    pub extents: Vec<Extent>,
}

impl RegionManifest {
    /// Resolve a live region by name (API.md §2 `resolve`).
    pub fn resolve(&self, name: &str) -> Option<ResolvedRegion> {
        let e = self
            .entries
            .iter()
            .find(|e| e.is_live() && e.name_bytes() == name.as_bytes())?;
        let off = e.extent_off as usize;
        let n = e.extent_n as usize;
        let extents = self.extents.get(off..off + n)?.to_vec();
        Some(ResolvedRegion {
            region_id: e.region_id,
            layout_version: e.layout_version,
            len: e.len,
            flags: e.flags,
            extents,
        })
    }
}

impl<M: GuestMem> Channel<M> {
    /// Seqlock-consistent manifest snapshot (API.md §4.2 reader discipline):
    /// read `generation` (even or retry), copy the manifest area, re-read
    /// `generation`; retry on change. After a snapshot restore the manifest
    /// is immediately valid (it is guest RAM) — no event replay needed.
    pub fn read_manifest(&self) -> Result<RegionManifest, WireError> {
        let gen_gpa = self.base + (OFF_MANIFEST + OFF_GENERATION) as u64;
        let area_gpa = self.base + OFF_MANIFEST as u64;
        let mut area = vec![0u8; MANIFEST_TOTAL_SIZE];
        for _ in 0..SEQLOCK_RETRIES {
            let g1 = self.gm.read_u64(gen_gpa)?;
            if g1 % 2 != 0 {
                continue; // writer mid-update
            }
            self.gm.read(area_gpa, &mut area)?;
            let g2 = self.gm.read_u64(gen_gpa)?;
            if g1 != g2 {
                continue;
            }
            let header = ManifestHeader::read_from(&area)?;
            header.validate()?;
            let mut entries = Vec::with_capacity(REGION_CAPACITY);
            for i in 0..REGION_CAPACITY {
                entries.push(RegionEntry::read_from(&area, i)?);
            }
            let n = header.extent_count as usize;
            debug_assert!(n <= EXTENT_CAPACITY); // validate() bounds it
            let mut extents = Vec::with_capacity(n);
            for i in 0..n {
                extents.push(Extent::read_from(&area, i)?);
            }
            // Entry extent ranges must sit inside the used pool.
            for e in entries.iter().filter(|e| e.is_live()) {
                e.validate_extents(&header)?;
            }
            return Ok(RegionManifest {
                header,
                entries,
                extents,
            });
        }
        Err(WireError::SeqlockLivelock)
    }

    /// Resolve + read a published region (API.md §2 `read_region`): walks the
    /// extent list, concatenating logically, reading each overlapping piece
    /// via `GuestMem::read`. This is the primitive the hypervisor's
    /// `ReadGuestMemory(region=..)` delegates to.
    pub fn read_region(
        &self,
        name: &str,
        offset: u64,
        buf: &mut [u8],
    ) -> Result<(), RegionReadError> {
        let manifest = self.read_manifest()?;
        let region = manifest
            .resolve(name)
            .ok_or(RegionReadError::NameNotFound)?;
        let want = buf.len() as u64;
        let end = offset
            .checked_add(want)
            .ok_or(RegionReadError::OutOfBounds)?;
        if end > region.len {
            return Err(RegionReadError::OutOfBounds);
        }
        // The extents must cover at least the region length.
        let covered: u64 = region.extents.iter().map(|x| x.len).sum();
        if covered < region.len {
            return Err(RegionReadError::OutOfBounds);
        }
        let mut remaining = buf;
        let mut to_skip = offset;
        for x in &region.extents {
            if remaining.is_empty() {
                break;
            }
            if to_skip >= x.len {
                to_skip -= x.len;
                continue;
            }
            let take = u64::min(x.len - to_skip, remaining.len() as u64) as usize;
            let (chunk, rest) = remaining.split_at_mut(take);
            self.gm.read(x.gpa + to_skip, chunk)?;
            remaining = rest;
            to_skip = 0;
        }
        debug_assert!(remaining.is_empty(), "coverage checked above");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::guestmem::MockGuestMem;
    use detguest_wire::header::{ChannelHeader, CHANNEL_SIZE, OFF_RESERVED};
    use detguest_wire::manifest::{init_manifest, writer_begin, writer_end, REGION_FLAG_DEAD};

    const BASE: u64 = 0x1000_0000;

    fn manifest_area(build: impl FnOnce(&mut [u8])) -> Channel<MockGuestMem> {
        let mut gm = MockGuestMem::with_zeroed(BASE, CHANNEL_SIZE);
        let mut hdr = [0u8; OFF_RESERVED];
        ChannelHeader::canonical().write_to(&mut hdr).unwrap();
        gm.write(BASE, &hdr).unwrap();
        let mut area = vec![0u8; MANIFEST_TOTAL_SIZE];
        init_manifest(&mut area).unwrap();
        build(&mut area);
        gm.write(BASE + OFF_MANIFEST as u64, &area).unwrap();
        Channel::attach(gm, BASE).unwrap()
    }

    fn put_region(
        area: &mut [u8],
        slot: usize,
        name: &[u8],
        len: u64,
        flags: u32,
        extent_off: u32,
        extents: &[Extent],
    ) {
        let e = RegionEntry {
            region_id: slot as u32,
            name_id: slot as u32 + 1,
            layout_version: 1,
            flags,
            gva: 0x7000_0000,
            len,
            extent_off,
            extent_n: extents.len() as u32,
            name: RegionEntry::pack_name(name).unwrap(),
        };
        e.write_to(area, slot).unwrap();
        for (i, x) in extents.iter().enumerate() {
            x.write_to(area, extent_off as usize + i).unwrap();
        }
        let mut h = ManifestHeader::read_from(area).unwrap();
        h.region_count += 1;
        h.extent_count = h.extent_count.max(extent_off + extents.len() as u32);
        h.generation = 2;
        h.write_to(area).unwrap();
    }

    #[test]
    fn read_manifest_resolves_live_and_skips_dead() {
        let ch = manifest_area(|area| {
            put_region(
                area,
                0,
                b"wram",
                0x100,
                0,
                0,
                &[Extent {
                    gpa: BASE,
                    len: 0x100,
                }],
            );
            put_region(
                area,
                1,
                b"dead",
                0x100,
                REGION_FLAG_DEAD,
                1,
                &[Extent {
                    gpa: BASE,
                    len: 0x100,
                }],
            );
        });
        let m = ch.read_manifest().unwrap();
        assert!(m.resolve("wram").is_some());
        assert!(m.resolve("dead").is_none(), "DEAD entries do not resolve");
        assert!(m.resolve("nope").is_none());
    }

    #[test]
    fn seqlock_odd_generation_then_recovery() {
        let mut ch = manifest_area(|area| {
            put_region(
                area,
                0,
                b"wram",
                0x10,
                0,
                0,
                &[Extent {
                    gpa: BASE,
                    len: 0x10,
                }],
            );
        });
        // Leave the generation odd: reader must NOT return a torn snapshot.
        let mut area = vec![0u8; MANIFEST_TOTAL_SIZE];
        ch.gm.read(BASE + OFF_MANIFEST as u64, &mut area).unwrap();
        writer_begin(&mut area).unwrap();
        ch.gm.write(BASE + OFF_MANIFEST as u64, &area).unwrap();
        assert_eq!(ch.read_manifest().unwrap_err(), WireError::SeqlockLivelock);
        // Writer finishes: reads succeed again.
        writer_end(&mut area).unwrap();
        ch.gm.write(BASE + OFF_MANIFEST as u64, &area).unwrap();
        assert!(ch.read_manifest().is_ok());
    }

    #[test]
    fn read_region_stitches_three_discontiguous_extents() {
        // M1 acceptance: a 3-extent region across a discontiguous mock
        // layout — GPA ranges that are NOT adjacent.
        const A: u64 = 0x4000_0000;
        const B: u64 = 0x5000_0000;
        const C: u64 = 0x6000_0000;
        let mut ch = manifest_area(|area| {
            put_region(
                area,
                0,
                b"wram",
                48,
                0,
                0,
                &[
                    Extent { gpa: A, len: 16 },
                    Extent { gpa: B, len: 24 },
                    Extent { gpa: C, len: 8 },
                ],
            );
        });
        ch.gm.add_segment(A, (0u8..16).collect());
        ch.gm.add_segment(B, (100u8..124).collect());
        ch.gm.add_segment(C, (200u8..208).collect());

        // Full read stitches all three.
        let mut buf = [0u8; 48];
        ch.read_region("wram", 0, &mut buf).unwrap();
        let expected: Vec<u8> = (0u8..16).chain(100..124).chain(200..208).collect();
        assert_eq!(&buf[..], &expected[..]);

        // Offset read crossing the A→B and B→C boundaries.
        let mut buf = [0u8; 20];
        ch.read_region("wram", 12, &mut buf).unwrap();
        assert_eq!(&buf[..], &expected[12..32]);
        let mut buf = [0u8; 10];
        ch.read_region("wram", 35, &mut buf).unwrap();
        assert_eq!(&buf[..], &expected[35..45]);

        // Bounds violations.
        let mut buf = [0u8; 8];
        assert_eq!(
            ch.read_region("wram", 44, &mut buf).unwrap_err(),
            RegionReadError::OutOfBounds
        );
        assert_eq!(
            ch.read_region("none", 0, &mut buf).unwrap_err(),
            RegionReadError::NameNotFound
        );
    }
}
