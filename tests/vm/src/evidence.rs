//! Durable, resumable evidence for the Ms5 seeded replay campaign.
//!
//! Each iteration is a separately synced JSON record. A manifest pins every
//! input that can change behavior, so continuation cannot mix workers,
//! binaries, images, revisions, schemas, seed mappings, or ranges.

use std::collections::BTreeSet;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

/// Current on-disk evidence schema.
pub const SCHEMA_VERSION: u32 = 1;
/// Seed generator/mapping identity.
pub const GENERATOR_VERSION: &str = "xorshift32-seed-base-plus-iteration-v1";

/// Configuration identity that must remain constant across resumed chunks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunIdentity {
    /// Stable runner identifier.
    pub runner_id: String,
    /// Guest-sdk revision.
    pub guest_sdk_sha: String,
    /// Hypervisor/worker revision or decoded-boundary provider revision.
    pub worker_sha: String,
    /// Reference-workload revision.
    pub workload_sha: String,
    /// Workload image digest.
    pub image_digest: String,
    /// Initramfs digest.
    pub initramfs_digest: String,
    /// Kernel build/digest identity.
    pub kernel_digest: String,
    /// Exact test-binary digest.
    pub test_binary_digest: String,
    /// First seed; iteration `i` uses `seed_base + i` modulo u32.
    pub seed_base: u32,
}

/// Requested consecutive iteration range `[start, start + count)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunRange {
    /// First iteration ID.
    pub start: u32,
    /// Number of iteration IDs.
    pub count: u32,
}

impl RunRange {
    /// Exclusive range end, rejecting overflow.
    pub fn end(self) -> io::Result<u32> {
        self.start
            .checked_add(self.count)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "iteration range overflow"))
    }
}

/// Four authoritative Ms5 equality surfaces.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SurfaceDigests {
    /// Canonical included guest-RAM ranges.
    pub final_guest_ram: String,
    /// Complete drained raw event stream.
    pub drained_events: String,
    /// Drop-counter structure.
    pub drop_counters: String,
    /// Canonical workload decision-echo LogLines.
    pub inject_decisions: String,
}

/// One completed iteration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IterationRecord {
    /// Schema version, always [`SCHEMA_VERSION`].
    pub schema_version: u32,
    /// Manifest run ID.
    pub run_id: String,
    /// Consecutive iteration ID.
    pub iteration: u32,
    /// Derived seed.
    pub seed: u32,
    /// Number of decoded PAD_SET records scheduled.
    pub input_burst_count: u32,
    /// Proceed, Platform, Workload decision counts.
    pub fault_class_counts: [u32; 3],
    /// Authoritative surface digests.
    pub surfaces: SurfaceDigests,
    /// External VerifyReplay/end-state evidence reference, when applicable.
    pub verify_replay_ref: String,
    /// Start timestamp in Unix milliseconds.
    pub started_unix_ms: u64,
    /// Wall duration in milliseconds.
    pub duration_ms: u64,
    /// `pass`, `fail`, or `divergence:<surface>`.
    pub outcome: String,
}

/// Run manifest, rewritten atomically after every record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    /// Schema version.
    pub schema_version: u32,
    /// Seed generator version.
    pub generator_version: String,
    /// Caller-assigned run ID.
    pub run_id: String,
    /// Full configuration identity.
    pub identity: RunIdentity,
    /// Full acceptance range; chunks must be consecutive subsets.
    pub requested: RunRange,
    /// Sorted completed iteration IDs.
    pub completed: Vec<u32>,
    /// Deterministic digest of ordered iteration JSON bytes.
    pub ordered_summary_digest: String,
}

/// Exclusive single-writer evidence directory.
pub struct EvidenceWriter {
    root: PathBuf,
    lock: PathBuf,
    manifest: Manifest,
}

impl EvidenceWriter {
    /// Create or resume a run, rejecting every identity/range drift.
    pub fn open(
        root: impl AsRef<Path>,
        run_id: &str,
        identity: RunIdentity,
        requested: RunRange,
    ) -> io::Result<Self> {
        requested.end()?;
        let root = root.as_ref().to_path_buf();
        fs::create_dir_all(root.join("records"))?;
        let lock = root.join("writer.lock");
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&lock)
            .map_err(|e| io::Error::new(e.kind(), format!("evidence writer lock: {e}")))?;

        let path = root.join("manifest.json");
        let manifest = if path.exists() {
            let existing: Manifest = serde_json::from_reader(File::open(&path)?)?;
            let expected = Manifest {
                schema_version: SCHEMA_VERSION,
                generator_version: GENERATOR_VERSION.into(),
                run_id: run_id.into(),
                identity,
                requested,
                completed: existing.completed.clone(),
                ordered_summary_digest: existing.ordered_summary_digest.clone(),
            };
            if existing != expected {
                let _ = fs::remove_file(&lock);
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "evidence identity or requested range drift",
                ));
            }
            existing
        } else {
            Manifest {
                schema_version: SCHEMA_VERSION,
                generator_version: GENERATOR_VERSION.into(),
                run_id: run_id.into(),
                identity,
                requested,
                completed: Vec::new(),
                ordered_summary_digest: String::new(),
            }
        };
        let mut writer = Self {
            root,
            lock,
            manifest,
        };
        writer.validate_completed()?;
        writer.sync_manifest()?;
        Ok(writer)
    }

    /// First iteration not yet recorded; gaps/duplicates are rejected.
    pub fn next_iteration(&self) -> u32 {
        self.manifest.requested.start + self.manifest.completed.len() as u32
    }

    /// Atomically append the next consecutive successful or failure record.
    pub fn append(&mut self, record: &IterationRecord) -> io::Result<()> {
        if record.schema_version != SCHEMA_VERSION
            || record.run_id != self.manifest.run_id
            || record.iteration != self.next_iteration()
            || record.iteration >= self.manifest.requested.end()?
            || record.seed
                != self
                    .manifest
                    .identity
                    .seed_base
                    .wrapping_add(record.iteration)
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "record overlap, gap, range, schema, run, or seed mismatch",
            ));
        }
        let bytes = serde_json::to_vec_pretty(record)?;
        atomic_write(
            &self
                .root
                .join("records")
                .join(format!("{:06}.json", record.iteration)),
            &bytes,
        )?;
        self.manifest.completed.push(record.iteration);
        self.manifest.ordered_summary_digest =
            ordered_digest(&self.root, &self.manifest.completed)?;
        self.sync_manifest()
    }

    /// Read the current manifest.
    pub fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    fn validate_completed(&self) -> io::Result<()> {
        let mut seen = BTreeSet::new();
        for (offset, &iteration) in self.manifest.completed.iter().enumerate() {
            let expected = self.manifest.requested.start + offset as u32;
            if iteration != expected || !seen.insert(iteration) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "manifest contains an iteration gap or duplicate",
                ));
            }
            let path = self
                .root
                .join("records")
                .join(format!("{iteration:06}.json"));
            let record: IterationRecord = serde_json::from_reader(File::open(path)?)?;
            if record.iteration != iteration {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "record filename/iteration mismatch",
                ));
            }
        }
        Ok(())
    }

    fn sync_manifest(&mut self) -> io::Result<()> {
        atomic_write(
            &self.root.join("manifest.json"),
            &serde_json::to_vec_pretty(&self.manifest)?,
        )
    }
}

impl Drop for EvidenceWriter {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.lock);
    }
}

fn atomic_write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let tmp = path.with_extension("tmp");
    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(&tmp)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    fs::rename(&tmp, path)?;
    File::open(path.parent().unwrap())?.sync_all()
}

fn ordered_digest(root: &Path, completed: &[u32]) -> io::Result<String> {
    // FNV-1a is used only as a stable evidence summary, never for security.
    let mut hash = 0xcbf29ce484222325u64;
    for iteration in completed {
        for byte in fs::read(root.join("records").join(format!("{iteration:06}.json")))? {
            hash ^= byte as u64;
            hash = hash.wrapping_mul(0x100000001b3);
        }
    }
    Ok(format!("fnv1a64:{hash:016x}"))
}

/// Current Unix time in milliseconds for evidence records.
pub fn unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "detguest-evidence-{name}-{}-{}",
            std::process::id(),
            unix_ms()
        ));
        let _ = fs::remove_dir_all(&path);
        path
    }

    fn identity() -> RunIdentity {
        RunIdentity {
            runner_id: "intel-1".into(),
            guest_sdk_sha: "g".into(),
            worker_sha: "w".into(),
            workload_sha: "r".into(),
            image_digest: "i".into(),
            initramfs_digest: "a".into(),
            kernel_digest: "k".into(),
            test_binary_digest: "t".into(),
            seed_base: 100,
        }
    }

    fn record(iteration: u32) -> IterationRecord {
        IterationRecord {
            schema_version: SCHEMA_VERSION,
            run_id: "run".into(),
            iteration,
            seed: 100 + iteration,
            input_burst_count: 2,
            fault_class_counts: [2, 1, 1],
            surfaces: SurfaceDigests {
                final_guest_ram: "a".into(),
                drained_events: "b".into(),
                drop_counters: "c".into(),
                inject_decisions: "d".into(),
            },
            verify_replay_ref: "external:x".into(),
            started_unix_ms: 1,
            duration_ms: 2,
            outcome: "pass".into(),
        }
    }

    #[test]
    fn resumes_only_at_consecutive_chunk_boundary() {
        let root = temp("resume");
        {
            let mut writer =
                EvidenceWriter::open(&root, "run", identity(), RunRange { start: 0, count: 3 })
                    .unwrap();
            writer.append(&record(0)).unwrap();
            writer.append(&record(1)).unwrap();
            assert!(writer.append(&record(1)).is_err(), "duplicate rejected");
        }
        let writer =
            EvidenceWriter::open(&root, "run", identity(), RunRange { start: 0, count: 3 })
                .unwrap();
        assert_eq!(writer.next_iteration(), 2);
        drop(writer);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn rejects_identity_range_gap_and_writer_overlap() {
        let root = temp("reject");
        let mut writer =
            EvidenceWriter::open(&root, "run", identity(), RunRange { start: 0, count: 2 })
                .unwrap();
        assert!(
            EvidenceWriter::open(&root, "run", identity(), RunRange { start: 0, count: 2 })
                .is_err()
        );
        assert!(writer.append(&record(1)).is_err(), "gap rejected");
        writer.append(&record(0)).unwrap();
        drop(writer);
        let mut changed = identity();
        changed.image_digest = "changed".into();
        assert!(
            EvidenceWriter::open(&root, "run", changed, RunRange { start: 0, count: 2 }).is_err()
        );
        assert!(
            EvidenceWriter::open(&root, "run", identity(), RunRange { start: 0, count: 3 })
                .is_err()
        );
        let _ = fs::remove_dir_all(root);
    }
}
