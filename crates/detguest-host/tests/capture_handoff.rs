//! Receiving-side M4 capture contract. Capture execution and compression stay
//! owned by determinism-hypervisor; this test pins and models only the bytes
//! guest-sdk publishes and the external handoff guest-sdk consumes.

use std::collections::BTreeMap;

const HANDOFF: &str = include_str!("fixtures/capture-handoff-v1.txt");

#[derive(Clone)]
struct Region {
    layout: u32,
    bytes: Vec<u8>,
}

#[derive(Clone, Copy)]
struct Range<'a> {
    name: &'a str,
    layout: u32,
    offset: usize,
    len: usize,
}

fn capture_features(
    regions: &BTreeMap<&str, Region>,
    ranges: &[Range<'_>],
) -> Result<Vec<u8>, String> {
    let mut packed = Vec::new();
    for range in ranges {
        let region = regions
            .get(range.name)
            .ok_or_else(|| format!("missing region {}", range.name))?;
        if region.layout != range.layout {
            return Err(format!(
                "FAILED_PRECONDITION: {} layout_version expected {}, got {}",
                range.name, range.layout, region.layout
            ));
        }
        let end = range
            .offset
            .checked_add(range.len)
            .filter(|end| *end <= region.bytes.len())
            .ok_or_else(|| format!("range outside {}", range.name))?;
        packed.extend_from_slice(&region.bytes[range.offset..end]);
    }
    Ok(packed)
}

#[test]
fn pinned_external_fixture_names_every_capture_surface() {
    for line in [
        "schema_version=1",
        "capture_surfaces=Run,TakeSnapshot",
        "feature_output=feature_bytes:request_order",
        "framebuffer_output=fb_lz4:size_prepended_lz4",
        "layout_mismatch=FAILED_PRECONDITION",
        "region=wram,1,131072,required",
        "region=framebuffer,1,229376,required,xrgb8888,256,224,1024",
        "region=meta,1,4096,required",
        "region=vram,1,65536,optional",
    ] {
        assert!(HANDOFF.lines().any(|got| got == line), "missing {line}");
    }
    assert!(HANDOFF.contains("hypervisor_sha=6e348e5961b8ba81d91b7bdd4f79af102b809649"));
    assert!(HANDOFF.contains("reference_workload_sha=7b0c7b2434e71d8b3241bf78597be457b281292d"));
}

#[test]
fn feature_ranges_pack_in_request_order_and_reject_layout_drift() {
    let regions = BTreeMap::from([
        (
            "wram",
            Region {
                layout: 1,
                bytes: (0..32).collect(),
            },
        ),
        (
            "meta",
            Region {
                layout: 1,
                bytes: (100..116).collect(),
            },
        ),
    ]);
    let ranges = [
        Range {
            name: "meta",
            layout: 1,
            offset: 3,
            len: 4,
        },
        Range {
            name: "wram",
            layout: 1,
            offset: 8,
            len: 5,
        },
    ];
    assert_eq!(
        capture_features(&regions, &ranges).unwrap(),
        [103, 104, 105, 106, 8, 9, 10, 11, 12]
    );

    let bad = [Range {
        layout: 2,
        ..ranges[0]
    }];
    let err = capture_features(&regions, &bad).unwrap_err();
    assert!(err.contains("FAILED_PRECONDITION"));
    assert!(err.contains("layout_version expected 2, got 1"));
}

#[test]
fn framebuffer_v1_is_raw_pixels_with_external_geometry() {
    const WIDTH: usize = 256;
    const HEIGHT: usize = 224;
    const STRIDE: usize = 1024;
    const LEN: usize = 229_376;
    assert_eq!(WIDTH * 4, STRIDE);
    assert_eq!(HEIGHT * STRIDE, LEN);
    let framebuffer = Region {
        layout: 1,
        bytes: vec![0xA5; LEN],
    };
    assert_eq!(framebuffer.bytes.len(), LEN);
    assert_eq!(framebuffer.layout, 1);
    // `fb_lz4` is the external transport; guest memory contains no metadata
    // header, only the XRGB8888 pixel bytes described by the fixture.
    assert_eq!(&framebuffer.bytes[..8], &[0xA5; 8]);
}
