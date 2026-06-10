//! `/etc/detguest/boot.toml` — parse + validation (API.md §7).
//!
//! This repo owns the format; the agent is its only parser. Any §7.2
//! violation is a boot fault (§7.3): the caller logs the detail, never emits
//! `Ready`, and powers off.

use std::collections::BTreeSet;

/// Major version this agent speaks (API.md §7.2: unknown major ⇒ boot fault).
pub const BOOT_TOML_MAJOR: i64 = 1;

/// One preconfigured workload entry (`[[unit]]`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unit {
    /// Dense id from 0, unique.
    pub id: u32,
    /// Absolute path inside the image.
    pub exec: String,
    /// argv (after argv0); never sent over the wire.
    pub args: Vec<String>,
    /// Initial LogLine mask (SetLogMask overrides). Default 0x1F.
    pub log_mask: u32,
    /// Harness control protocol (M3+; parsed now, driven later).
    pub control: Option<UnitControl>,
}

/// `[unit.control]` (API.md §7.1) — present ⇒ the unit speaks the harness
/// control protocol and the agent drives its leg (ARCHITECTURE.md §4.2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnitControl {
    /// Open identifier; v1 defines only `refwork-ctl`.
    pub protocol: String,
    /// Must equal the version the agent speaks.
    pub proto_version: u32,
    /// The LoadGame.dev_path the agent sends (required for refwork-ctl).
    pub game_dev: Option<String>,
}

/// `[[expected_region]]` — the READY gate (ARCHITECTURE.md §4.1/§4.2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExpectedRegion {
    /// Region name (≤ 56 bytes, the manifest cap).
    pub name: String,
    /// Must match the manifest entry exactly; mismatch is a boot fault.
    pub layout_version: u32,
}

/// The parsed, validated boot manifest.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BootManifest {
    /// `[autostart].unit` if configured.
    pub autostart_unit: Option<u32>,
    /// All `[[unit]]` entries, sorted by id (dense from 0).
    pub units: Vec<Unit>,
    /// The READY gate region list (may be empty).
    pub expected_regions: Vec<ExpectedRegion>,
}

impl BootManifest {
    /// Find a unit by id.
    pub fn unit(&self, id: u32) -> Option<&Unit> {
        self.units.iter().find(|u| u.id == id)
    }
}

/// A §7.2 violation: the detail string goes out as the boot-fault LogLine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BootFault(pub String);

impl std::fmt::Display for BootFault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "boot.toml fault: {}", self.0)
    }
}

fn fault(msg: impl Into<String>) -> BootFault {
    BootFault(msg.into())
}

/// Parse and §7.2-validate boot.toml contents.
pub fn parse(text: &str) -> Result<BootManifest, BootFault> {
    let doc: toml::Value = text
        .parse()
        .map_err(|e| fault(format!("parse error: {e}")))?;
    let table = doc
        .as_table()
        .ok_or_else(|| fault("top level must be a table"))?;

    // boot_toml_version: required; unknown major ⇒ loud fault (§7.2).
    let version = table
        .get("boot_toml_version")
        .ok_or_else(|| fault("missing required boot_toml_version"))?
        .as_integer()
        .ok_or_else(|| fault("boot_toml_version must be an integer"))?;
    if version != BOOT_TOML_MAJOR {
        return Err(fault(format!(
            "unknown boot_toml_version major {version} (agent speaks {BOOT_TOML_MAJOR})"
        )));
    }

    // [[unit]]
    let mut units = Vec::new();
    if let Some(v) = table.get("unit") {
        let arr = v
            .as_array()
            .ok_or_else(|| fault("unit must be an array of tables"))?;
        for (i, u) in arr.iter().enumerate() {
            let t = u
                .as_table()
                .ok_or_else(|| fault(format!("unit[{i}] must be a table")))?;
            let id = t
                .get("id")
                .and_then(|x| x.as_integer())
                .ok_or_else(|| fault(format!("unit[{i}]: missing integer id")))?;
            if id < 0 || id > u32::MAX as i64 {
                return Err(fault(format!("unit[{i}]: id out of range")));
            }
            let exec = t
                .get("exec")
                .and_then(|x| x.as_str())
                .ok_or_else(|| fault(format!("unit[{i}]: missing string exec")))?;
            if !exec.starts_with('/') {
                return Err(fault(format!(
                    "unit[{i}]: exec must be an absolute path inside the image (got {exec:?})"
                )));
            }
            let args = match t.get("args") {
                None => Vec::new(),
                Some(a) => a
                    .as_array()
                    .ok_or_else(|| fault(format!("unit[{i}]: args must be an array")))?
                    .iter()
                    .map(|x| {
                        x.as_str()
                            .map(str::to_owned)
                            .ok_or_else(|| fault(format!("unit[{i}]: args must be strings")))
                    })
                    .collect::<Result<_, _>>()?,
            };
            let log_mask = match t.get("log_mask") {
                None => 0x1F,
                Some(x) => x
                    .as_integer()
                    .filter(|m| (0..=u32::MAX as i64).contains(m))
                    .ok_or_else(|| fault(format!("unit[{i}]: log_mask must be a u32")))?
                    as u32,
            };
            let control = match t.get("control") {
                None => None,
                Some(c) => {
                    let ct = c
                        .as_table()
                        .ok_or_else(|| fault(format!("unit[{i}].control must be a table")))?;
                    let protocol = ct
                        .get("protocol")
                        .and_then(|x| x.as_str())
                        .ok_or_else(|| fault(format!("unit[{i}].control: missing protocol")))?
                        .to_owned();
                    let proto_version = ct
                        .get("proto_version")
                        .and_then(|x| x.as_integer())
                        .filter(|v| (0..=u32::MAX as i64).contains(v))
                        .ok_or_else(|| {
                            fault(format!("unit[{i}].control: missing u32 proto_version"))
                        })? as u32;
                    let game_dev = ct
                        .get("game_dev")
                        .and_then(|x| x.as_str())
                        .map(str::to_owned);
                    if protocol == "refwork-ctl" && game_dev.is_none() {
                        return Err(fault(format!(
                            "unit[{i}].control: game_dev required for refwork-ctl (§7.2)"
                        )));
                    }
                    Some(UnitControl {
                        protocol,
                        proto_version,
                        game_dev,
                    })
                }
            };
            units.push(Unit {
                id: id as u32,
                exec: exec.to_owned(),
                args,
                log_mask,
                control,
            });
        }
    }
    // ids dense from 0 and unique (§7.2).
    let mut ids: Vec<u32> = units.iter().map(|u| u.id).collect();
    ids.sort_unstable();
    for (want, got) in ids.iter().enumerate() {
        if *got != want as u32 {
            return Err(fault(format!(
                "unit ids must be dense from 0 and unique (got {ids:?})"
            )));
        }
    }
    units.sort_by_key(|u| u.id);

    // [autostart]
    let autostart_unit = match table.get("autostart") {
        None => None,
        Some(a) => {
            let t = a
                .as_table()
                .ok_or_else(|| fault("autostart must be a table"))?;
            let id = t
                .get("unit")
                .and_then(|x| x.as_integer())
                .filter(|v| (0..=u32::MAX as i64).contains(v))
                .ok_or_else(|| fault("autostart: missing u32 unit"))? as u32;
            if !units.iter().any(|u| u.id == id) {
                return Err(fault(format!(
                    "autostart references nonexistent unit id {id}"
                )));
            }
            Some(id)
        }
    };

    // [[expected_region]]
    let mut expected_regions = Vec::new();
    if let Some(v) = table.get("expected_region") {
        let arr = v
            .as_array()
            .ok_or_else(|| fault("expected_region must be an array of tables"))?;
        let mut seen = BTreeSet::new();
        for (i, r) in arr.iter().enumerate() {
            let t = r
                .as_table()
                .ok_or_else(|| fault(format!("expected_region[{i}] must be a table")))?;
            let name = t
                .get("name")
                .and_then(|x| x.as_str())
                .ok_or_else(|| fault(format!("expected_region[{i}]: missing string name")))?;
            if name.len() > detguest_wire::manifest::MAX_REGION_NAME {
                return Err(fault(format!(
                    "expected_region[{i}]: name exceeds the 56-byte manifest cap"
                )));
            }
            if !seen.insert(name.to_owned()) {
                return Err(fault(format!("duplicate expected_region name {name:?}")));
            }
            let layout_version = t
                .get("layout_version")
                .and_then(|x| x.as_integer())
                .filter(|v| (0..=u32::MAX as i64).contains(v))
                .ok_or_else(|| fault(format!("expected_region[{i}]: missing u32 layout_version")))?
                as u32;
            expected_regions.push(ExpectedRegion {
                name: name.to_owned(),
                layout_version,
            });
        }
    }

    Ok(BootManifest {
        autostart_unit,
        units,
        expected_regions,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const GOOD: &str = r#"
boot_toml_version = 1

[autostart]
unit = 0

[[unit]]
id = 0
exec = "/opt/autostart-trivial"
log_mask = 0x1F

[[unit]]
id = 1
exec = "/opt/print-lines"
args = ["--fast"]

[[expected_region]]
name = "wram"
layout_version = 1
"#;

    #[test]
    fn good_manifest_parses() {
        let m = parse(GOOD).unwrap();
        assert_eq!(m.autostart_unit, Some(0));
        assert_eq!(m.units.len(), 2);
        assert_eq!(m.unit(0).unwrap().exec, "/opt/autostart-trivial");
        assert_eq!(m.unit(1).unwrap().args, vec!["--fast"]);
        assert_eq!(m.unit(0).unwrap().log_mask, 0x1F);
        assert_eq!(m.expected_regions.len(), 1);
    }

    #[test]
    fn minimal_manifest_no_autostart() {
        let m = parse("boot_toml_version = 1\n").unwrap();
        assert_eq!(m.autostart_unit, None);
        assert!(m.units.is_empty());
        assert!(m.expected_regions.is_empty());
    }

    #[test]
    fn faults_per_7_2() {
        // missing version
        assert!(parse("").unwrap_err().0.contains("boot_toml_version"));
        // unknown major
        assert!(parse("boot_toml_version = 2\n")
            .unwrap_err()
            .0
            .contains("unknown"));
        // parse error
        assert!(parse("boot_toml_version = ")
            .unwrap_err()
            .0
            .contains("parse error"));
        // non-dense ids
        let e = parse("boot_toml_version = 1\n[[unit]]\nid = 1\nexec = \"/x\"\n").unwrap_err();
        assert!(e.0.contains("dense"), "{e:?}");
        // duplicate ids
        let e = parse(
            "boot_toml_version = 1\n[[unit]]\nid = 0\nexec = \"/x\"\n[[unit]]\nid = 0\nexec = \"/y\"\n",
        )
        .unwrap_err();
        assert!(e.0.contains("dense"), "{e:?}");
        // relative exec path
        let e = parse("boot_toml_version = 1\n[[unit]]\nid = 0\nexec = \"x\"\n").unwrap_err();
        assert!(e.0.contains("absolute"), "{e:?}");
        // autostart to nonexistent unit
        let e = parse("boot_toml_version = 1\n[autostart]\nunit = 3\n").unwrap_err();
        assert!(e.0.contains("nonexistent"), "{e:?}");
        // duplicate region names
        let e = parse(
            "boot_toml_version = 1\n[[expected_region]]\nname = \"a\"\nlayout_version = 1\n[[expected_region]]\nname = \"a\"\nlayout_version = 1\n",
        )
        .unwrap_err();
        assert!(e.0.contains("duplicate"), "{e:?}");
        // over-long region name
        let long = "x".repeat(57);
        let e = parse(&format!(
            "boot_toml_version = 1\n[[expected_region]]\nname = \"{long}\"\nlayout_version = 1\n"
        ))
        .unwrap_err();
        assert!(e.0.contains("56-byte"), "{e:?}");
        // control without game_dev for refwork-ctl
        let e = parse(
            "boot_toml_version = 1\n[[unit]]\nid = 0\nexec = \"/x\"\n[unit.control]\nprotocol = \"refwork-ctl\"\nproto_version = 1\n",
        )
        .unwrap_err();
        assert!(e.0.contains("game_dev"), "{e:?}");
    }

    #[test]
    fn spec_example_shape_parses() {
        // The API.md §7.1 example (trimmed to the fields the agent reads).
        let m = parse(
            r#"
boot_toml_version = 1
[autostart]
unit = 0
[[unit]]
id = 0
exec = "/usr/bin/refwork-harness"
args = ["--config", "/etc/refwork/harness.toml"]
log_mask = 0x1F
[unit.control]
protocol = "refwork-ctl"
proto_version = 1
game_dev = "/dev/vdb"
[[expected_region]]
name = "wram"
layout_version = 1
[[expected_region]]
name = "framebuffer"
layout_version = 1
[[expected_region]]
name = "meta"
layout_version = 1
"#,
        )
        .unwrap();
        let c = m.unit(0).unwrap().control.as_ref().unwrap();
        assert_eq!(c.protocol, "refwork-ctl");
        assert_eq!(c.game_dev.as_deref(), Some("/dev/vdb"));
        assert_eq!(m.expected_regions.len(), 3);
    }
}
