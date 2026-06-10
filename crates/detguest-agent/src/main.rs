//! detguest-agent binary — PID 1 of the deterministic guest image
//! (ARCHITECTURE.md §4). `--check` prints the version and exits 0 (used by
//! image assembly smoke tests); anything else runs the boot sequence.

fn main() {
    if std::env::args().any(|a| a == "--check") {
        println!(
            "detguest-agent {} proto {}",
            env!("CARGO_PKG_VERSION"),
            detguest_wire::PROTO_VERSION
        );
        return;
    }
    detguest_agent::runtime::run()
}
