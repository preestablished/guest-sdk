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
        _ => exercise_once(),
    }
    std::process::exit(EXIT_CODE);
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
