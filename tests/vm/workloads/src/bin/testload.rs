//! Deterministic M3 SDK exercise workload.
//!
//! This binary links `detguest-sdk` and touches each public M3 workload API
//! once. It is intentionally standalone-safe: outside the agent, `init()`
//! returns `NoChannel` and the rest of the SDK is a deterministic no-op, so
//! hosted builds can compile it without KVM or a guest image.
#![forbid(unsafe_code)]

use detguest_sdk::{self as sdk, LogLevel};

const EXIT_CODE: i32 = 0;
const SPAM_LOGS: u32 = 80_000;
const SPAM_ASSERTS: u32 = 20_000;

fn main() {
    match std::env::args().nth(1).as_deref() {
        Some("--spam-logs") => spam_logs(),
        Some("--spam-asserts") => spam_asserts(),
        Some("--inject-roundtrip") => inject_roundtrip(),
        _ => exercise_once(),
    }
    std::process::exit(EXIT_CODE);
}

/// Canonical Ms5 live-inject fixture. The point order and log schema are
/// versioned so the host can correlate `(iseq, name_id)` query order with
/// the workload-observed return without exposing iseq through the SDK API.
fn inject_roundtrip() {
    let _ = sdk::init();
    const POINTS: [&str; 6] = [
        "ms5.frame.begin",
        "ms5.io.read",
        "ms5.frame.end",
        "ms5.frame.begin",
        "ms5.io.write",
        "ms5.frame.end",
    ];

    for (occurrence, point) in POINTS.into_iter().enumerate() {
        let decision = sdk::inject_point(point);
        let (class, kind, arg) = match decision {
            sdk::FaultDecision::Proceed => ("proceed", 0, 0),
            sdk::FaultDecision::Platform { kind, arg } => ("platform", kind, arg),
            sdk::FaultDecision::Workload { kind, arg } => ("workload", kind, arg),
        };
        sdk::log_line(
            LogLevel::Info,
            &format!(
                "ms5.inject.v1 occurrence={occurrence} point={point} class={class} kind={kind} arg={arg}"
            ),
        );

        // Keep the pv-pad path live in the same trajectory. Frame boundaries
        // follow the two stable frame.end points.
        if point == "ms5.io.read" || point == "ms5.io.write" {
            let pad0 = sdk::poll_input(0);
            sdk::log_line(
                LogLevel::Info,
                &format!("ms5.input.v1 occurrence={occurrence} port=0 value={pad0}"),
            );
        }
        if point == "ms5.frame.end" {
            sdk::frame_mark();
        }
    }
}

fn exercise_once() {
    let _ = sdk::init();

    sdk::declare_reachable("testload.main.declared");
    sdk::expect_reachable("testload.main.entered");
    sdk::assert_always(true, "testload.assert.true", format_args!(""));
    sdk::assert_always(
        false,
        "testload.assert.false",
        format_args!("expected deterministic violation"),
    );
    sdk::coverage_beacon(1);
    sdk::log_line(LogLevel::Info, "testload: sdk api pass");

    let pad0 = sdk::poll_input(0);
    sdk::frame_mark();
    sdk::quiesce_check();

    sdk::log_line(LogLevel::Debug, &format!("testload: pad0={pad0}"));
}

fn spam_logs() {
    let _ = sdk::init();
    for i in 0..SPAM_LOGS {
        sdk::log_line(LogLevel::Info, &format!("testload spam log {i:05}"));
    }
    sdk::frame_mark();
}

fn spam_asserts() {
    let _ = sdk::init();
    for i in 0..SPAM_ASSERTS {
        let name: &'static str = Box::leak(format!("testload.spam.assert.{i:05}").into_boxed_str());
        sdk::assert_always(false, name, format_args!("i={i}"));
    }
    sdk::frame_mark();
}
