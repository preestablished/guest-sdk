//! Deterministic M3 SDK exercise workload.
//!
//! This binary links `detguest-sdk` and touches each public M3 workload API
//! once. It is intentionally standalone-safe: outside the agent, `init()`
//! returns `NoChannel` and the rest of the SDK is a deterministic no-op, so
//! hosted builds can compile it without KVM or a guest image.
#![forbid(unsafe_code)]

use detguest_sdk::{self as sdk, LogLevel};

const EXIT_CODE: i32 = 0;

fn main() {
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
    std::process::exit(EXIT_CODE);
}
