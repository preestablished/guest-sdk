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
    /// The logical game device name (required for refwork-ctl). Sent
    /// verbatim as LoadGame.dev_path unless `game_source` overrides it.
    pub game_dev: Option<String>,
    /// Where the game bytes come from. `Some(PvBlk)` ⇒ the agent
    /// materializes the image from the pv-blk MMIO device to
    /// `pvblk::GAME_IMG_PATH` before LoadGame and sends that path; `None`
    /// ⇒ `game_dev` is sent verbatim (the pre-materialization behavior).
    pub game_source: Option<GameSource>,
}

/// `[unit.control].game_source` values (API.md §7.2; an enum so unknown
/// values die in the parser, not deep in the boot sequence).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GameSource {
    /// Materialize from the pv-blk MMIO device.
    PvBlk,
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
///
/// Parsing uses the in-crate deterministic TOML-subset parser (`tiny`),
/// not the `toml` crate: that crate's parser seeds a hash table via
/// `getrandom(2)` at parse time, which in-guest is entropy consumption — a
/// P0 violation of ARCHITECTURE.md §7 rule 2. The subset covers exactly the
/// §7.1 schema: comments, `[table]` / `[[array-of-tables]]` / dotted
/// `[unit.control]` headers, basic strings, integers (incl. `0x` hex), and
/// inline string arrays.
pub fn parse(text: &str) -> Result<BootManifest, BootFault> {
    let doc: tiny::Value = tiny::parse_doc(text).map_err(|e| fault(format!("parse error: {e}")))?;
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
                    // TODO(M4): when the §4.2 protocol leg lands, validate
                    // proto_version equals the version the agent speaks
                    // (§7.2) at boot rather than at Hello time.
                    let game_dev = ct
                        .get("game_dev")
                        .and_then(|x| x.as_str())
                        .map(str::to_owned);
                    if protocol == "refwork-ctl" && game_dev.is_none() {
                        return Err(fault(format!(
                            "unit[{i}].control: game_dev required for refwork-ctl (§7.2)"
                        )));
                    }
                    let game_source = match ct.get("game_source") {
                        None => None,
                        Some(v) => match v.as_str() {
                            Some("pv-blk") => Some(GameSource::PvBlk),
                            Some(other) => {
                                return Err(fault(format!(
                                    "unit[{i}].control: unknown game_source {other:?} (v1 knows \"pv-blk\")"
                                )))
                            }
                            None => {
                                return Err(fault(format!(
                                    "unit[{i}].control: game_source must be a string"
                                )))
                            }
                        },
                    };
                    Some(UnitControl {
                        protocol,
                        proto_version,
                        game_dev,
                        game_source,
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

/// Deterministic TOML-subset parser for the API.md §7.1 schema.
///
/// No hash tables anywhere (BTreeMap only — §7 rule 2 forbids the std
/// HashMap default hasher and any entropy draw), no dependencies, total over
/// arbitrary input (errors, never panics). Supported: `#` comments, blank
/// lines, `[a]` / `[a.b]` table headers (dotted segments descend through the
/// LAST element of an array-of-tables, per TOML), `[[a]]` array-of-tables,
/// bare keys, basic `"strings"` with `\\ \" \n \t` escapes, decimal and
/// `0x` hex integers, and single-line `["string", ...]` arrays.
mod tiny {
    use std::collections::BTreeMap;

    /// The value tree (accessor surface mirrors `toml::Value`).
    #[derive(Debug, Clone, PartialEq)]
    pub enum Value {
        /// Basic string.
        Str(String),
        /// Integer.
        Int(i64),
        /// Array (strings or tables in this subset).
        Array(Vec<Value>),
        /// Table.
        Table(BTreeMap<String, Value>),
    }

    impl Value {
        pub fn as_table(&self) -> Option<&BTreeMap<String, Value>> {
            match self {
                Value::Table(t) => Some(t),
                _ => None,
            }
        }
        pub fn as_integer(&self) -> Option<i64> {
            match self {
                Value::Int(i) => Some(*i),
                _ => None,
            }
        }
        pub fn as_str(&self) -> Option<&str> {
            match self {
                Value::Str(s) => Some(s),
                _ => None,
            }
        }
        pub fn as_array(&self) -> Option<&Vec<Value>> {
            match self {
                Value::Array(a) => Some(a),
                _ => None,
            }
        }
    }

    /// Strip a `#` comment (respecting `"` strings) and trim.
    fn strip_comment(line: &str) -> &str {
        let mut in_str = false;
        let mut escaped = false;
        for (i, c) in line.char_indices() {
            match c {
                '\\' if in_str && !escaped => {
                    escaped = true;
                    continue;
                }
                '"' if !escaped => in_str = !in_str,
                '#' if !in_str => return &line[..i],
                _ => {}
            }
            escaped = false;
        }
        line
    }

    fn parse_string(s: &str) -> Result<(String, &str), String> {
        let rest = s
            .strip_prefix('"')
            .ok_or_else(|| format!("expected string at {s:?}"))?;
        let mut out = String::new();
        let mut chars = rest.char_indices();
        while let Some((i, c)) = chars.next() {
            match c {
                '"' => return Ok((out, &rest[i + 1..])),
                '\\' => match chars.next() {
                    Some((_, '"')) => out.push('"'),
                    Some((_, '\\')) => out.push('\\'),
                    Some((_, 'n')) => out.push('\n'),
                    Some((_, 't')) => out.push('\t'),
                    other => return Err(format!("unsupported escape {other:?}")),
                },
                _ => out.push(c),
            }
        }
        Err("unterminated string".into())
    }

    fn parse_int(s: &str) -> Result<i64, String> {
        let t = s.trim();
        let (neg, t) = match t.strip_prefix('-') {
            Some(r) => (true, r),
            None => (false, t.strip_prefix('+').unwrap_or(t)),
        };
        let digits = t.replace('_', "");
        let v = if let Some(hex) = digits
            .strip_prefix("0x")
            .or_else(|| digits.strip_prefix("0X"))
        {
            i64::from_str_radix(hex, 16)
        } else {
            digits.parse::<i64>()
        }
        .map_err(|e| format!("bad integer {s:?}: {e}"))?;
        Ok(if neg { -v } else { v })
    }

    fn parse_value(s: &str) -> Result<Value, String> {
        let t = s.trim();
        if t.starts_with('"') {
            let (v, rest) = parse_string(t)?;
            if !rest.trim().is_empty() {
                return Err(format!("trailing junk after string: {rest:?}"));
            }
            return Ok(Value::Str(v));
        }
        if let Some(inner) = t.strip_prefix('[') {
            let inner = inner
                .strip_suffix(']')
                .ok_or_else(|| format!("unterminated array {t:?}"))?;
            let mut items = Vec::new();
            let mut rest = inner.trim();
            while !rest.is_empty() {
                let (v, after) = parse_string(rest)?;
                items.push(Value::Str(v));
                rest = after.trim_start();
                if let Some(r) = rest.strip_prefix(',') {
                    rest = r.trim_start();
                } else if !rest.is_empty() {
                    return Err(format!("expected ',' in array near {rest:?}"));
                }
            }
            return Ok(Value::Array(items));
        }
        parse_int(t).map(Value::Int)
    }

    fn valid_key(k: &str) -> bool {
        !k.is_empty()
            && k.chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    }

    /// Navigate to the table at `path` (creating tables; descending through
    /// the last element of arrays-of-tables), returning a mutable ref.
    fn navigate<'a>(
        root: &'a mut BTreeMap<String, Value>,
        path: &[&str],
    ) -> Result<&'a mut BTreeMap<String, Value>, String> {
        let mut cur = root;
        for seg in path {
            if !valid_key(seg) {
                return Err(format!("invalid table name segment {seg:?}"));
            }
            let entry = cur
                .entry((*seg).to_owned())
                .or_insert_with(|| Value::Table(BTreeMap::new()));
            cur = match entry {
                Value::Table(t) => t,
                Value::Array(a) => match a.last_mut() {
                    Some(Value::Table(t)) => t,
                    _ => return Err(format!("{seg} is not a table array")),
                },
                _ => return Err(format!("{seg} is not a table")),
            };
        }
        Ok(cur)
    }

    /// Parse a document into a root table.
    pub fn parse_doc(text: &str) -> Result<Value, String> {
        let mut root: BTreeMap<String, Value> = BTreeMap::new();
        // Path of the currently open table header.
        let mut cur_path: Vec<String> = Vec::new();
        for (lineno, raw) in text.lines().enumerate() {
            let line = strip_comment(raw).trim();
            let err = |m: String| format!("line {}: {m}", lineno + 1);
            if line.is_empty() {
                continue;
            }
            if let Some(h) = line.strip_prefix("[[") {
                let name = h
                    .strip_suffix("]]")
                    .ok_or_else(|| err("malformed [[header]]".into()))?
                    .trim();
                let segs: Vec<&str> = name.split('.').map(str::trim).collect();
                let (last, parents) = segs
                    .split_last()
                    .ok_or_else(|| err("empty [[header]]".into()))?;
                if !valid_key(last) {
                    return Err(err(format!("invalid table name {last:?}")));
                }
                let parent = navigate(&mut root, parents).map_err(err)?;
                let arr = parent
                    .entry((*last).to_owned())
                    .or_insert_with(|| Value::Array(Vec::new()));
                match arr {
                    Value::Array(a) => a.push(Value::Table(BTreeMap::new())),
                    _ => return Err(err(format!("{last} is not an array of tables"))),
                }
                cur_path = segs.iter().map(|s| (*s).to_owned()).collect();
            } else if let Some(h) = line.strip_prefix('[') {
                let name = h
                    .strip_suffix(']')
                    .ok_or_else(|| err("malformed [header]".into()))?
                    .trim();
                let segs: Vec<&str> = name.split('.').map(str::trim).collect();
                navigate(&mut root, &segs).map_err(&err)?;
                cur_path = segs.iter().map(|s| (*s).to_owned()).collect();
            } else if let Some(eq) = line.find('=') {
                let key = line[..eq].trim();
                if !valid_key(key) {
                    return Err(err(format!("invalid key {key:?}")));
                }
                let value = parse_value(&line[eq + 1..]).map_err(&err)?;
                let path: Vec<&str> = cur_path.iter().map(String::as_str).collect();
                let table = navigate(&mut root, &path).map_err(&err)?;
                if table.insert(key.to_owned(), value).is_some() {
                    return Err(err(format!("duplicate key {key:?}")));
                }
            } else {
                return Err(err(format!("unrecognized line {line:?}")));
            }
        }
        Ok(Value::Table(root))
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn determinism_identical_reparse() {
            let doc = "a = 1\n[t]\nb = \"x\"\n[[arr]]\nc = 0x1F\n";
            assert_eq!(parse_doc(doc).unwrap(), parse_doc(doc).unwrap());
        }

        #[test]
        fn comments_strings_escapes_hex() {
            let v = parse_doc(
                "# top\nkey = \"a # not comment \\\"q\\\"\" # trailing\nhex = 0x1F\nneg = -3\n",
            )
            .unwrap();
            let t = v.as_table().unwrap();
            assert_eq!(t["key"].as_str(), Some("a # not comment \"q\""));
            assert_eq!(t["hex"].as_integer(), Some(0x1F));
            assert_eq!(t["neg"].as_integer(), Some(-3));
        }

        #[test]
        fn dotted_header_attaches_to_last_array_element() {
            let v =
                parse_doc("[[unit]]\nid = 0\n[[unit]]\nid = 1\n[unit.control]\nprotocol = \"p\"\n")
                    .unwrap();
            let units = v.as_table().unwrap()["unit"].as_array().unwrap();
            assert!(units[0].as_table().unwrap().get("control").is_none());
            let c = units[1].as_table().unwrap()["control"].as_table().unwrap();
            assert_eq!(c["protocol"].as_str(), Some("p"));
        }

        #[test]
        fn string_arrays() {
            let v = parse_doc("args = [\"--a\", \"b c\"]\nempty = []\n").unwrap();
            let t = v.as_table().unwrap();
            assert_eq!(t["args"].as_array().unwrap().len(), 2);
            assert!(t["empty"].as_array().unwrap().is_empty());
        }

        #[test]
        fn errors_never_panic() {
            for bad in [
                "key",
                "= 1",
                "[unterminated",
                "[[x]",
                "k = \"unterminated",
                "k = [\"a\" \"b\"]",
                "k = zzz",
                "k = 1\nk = 2",
                "[5bad key]",
                "k = \"a\" junk",
            ] {
                assert!(parse_doc(bad).is_err(), "{bad:?} must error");
            }
        }
    }
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
        // unknown game_source value
        let e = parse(
            "boot_toml_version = 1\n[[unit]]\nid = 0\nexec = \"/x\"\n[unit.control]\nprotocol = \"refwork-ctl\"\nproto_version = 1\ngame_dev = \"/dev/vdb\"\ngame_source = \"floppy\"\n",
        )
        .unwrap_err();
        assert!(e.0.contains("game_source"), "{e:?}");
        assert!(e.0.contains("floppy"), "{e:?}");
        // non-string game_source
        let e = parse(
            "boot_toml_version = 1\n[[unit]]\nid = 0\nexec = \"/x\"\n[unit.control]\nprotocol = \"refwork-ctl\"\nproto_version = 1\ngame_dev = \"/dev/vdb\"\ngame_source = 1\n",
        )
        .unwrap_err();
        assert!(e.0.contains("game_source"), "{e:?}");
    }

    #[test]
    fn game_source_pv_blk_parses_and_defaults_to_none() {
        let m = parse(
            "boot_toml_version = 1\n[[unit]]\nid = 0\nexec = \"/x\"\n[unit.control]\nprotocol = \"refwork-ctl\"\nproto_version = 1\ngame_dev = \"/dev/vdb\"\ngame_source = \"pv-blk\"\n",
        )
        .unwrap();
        let c = m.unit(0).unwrap().control.as_ref().unwrap();
        assert_eq!(c.game_source, Some(GameSource::PvBlk));
        assert_eq!(c.game_dev.as_deref(), Some("/dev/vdb"));

        // Absent field => None: the pre-materialization behavior, so every
        // committed fixture (m2/m4/m9) parses unchanged.
        let m = parse(
            "boot_toml_version = 1\n[[unit]]\nid = 0\nexec = \"/x\"\n[unit.control]\nprotocol = \"refwork-ctl\"\nproto_version = 1\ngame_dev = \"/dev/vdb\"\n",
        )
        .unwrap();
        assert_eq!(
            m.unit(0).unwrap().control.as_ref().unwrap().game_source,
            None
        );
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

    #[test]
    fn committed_m9_refwork_contract_manifest_parses() {
        let m = parse(include_str!("../../../image/boot.toml.m9-refwork-contract")).unwrap();
        assert_eq!(m.autostart_unit, Some(0));
        let unit = m.unit(0).unwrap();
        assert_eq!(unit.exec, "/opt/m9-refwork-contract");
        let control = unit.control.as_ref().unwrap();
        assert_eq!(control.protocol, "refwork-ctl");
        assert_eq!(control.proto_version, 1);
        assert_eq!(control.game_dev.as_deref(), Some("/dev/vdb"));
        let regions: Vec<_> = m
            .expected_regions
            .iter()
            .map(|region| (region.name.as_str(), region.layout_version))
            .collect();
        assert_eq!(regions, vec![("wram", 1), ("framebuffer", 1), ("meta", 1)]);
    }
}
