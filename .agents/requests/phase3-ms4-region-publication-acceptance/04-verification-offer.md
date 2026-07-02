# What The Bridge Provides For Verification

The rom-operator-bridge is a deployed, working operator surface over the
worker gRPC API, and we will run the downstream half of any verification you
need. As of 2026-07-02 it provides, verified end-to-end in a real browser:

- session lifecycle against real `dh-workerd` slots (RestoreSnapshot on
  start, DestroyVm on stop, pause/resume);
- frame preview via `GetFramebuffer` with honest state reporting — a healthy
  session with no readable frame renders a calm "No Frame Yet", a missing
  backend renders a distinct outage state, and every worker RPC failure is
  logged with its gRPC code and message (`journalctl -u rom-operator-bridge`,
  WARN level) — so a misbehaving region surfaces with a precise error, not a
  silent hang;
- scheduled pad input through the hypervisor's `InjectInputs` path (DH-2),
  driven from the browser or curl.

## Standing Offer

When a READY snapshot with the real workload exists (refwork M4 + your Ms4),
tell us the snapshot ref channel to update (the private handoff env file) and
we will:

1. restart the deployed worker/bridge cleanly (we own that procedure,
   including the lease-invalidation caveat);
2. run start → `GetFramebuffer` → browser-preview verification and report
   results with artifact-grade precision back into this request directory;
3. exercise scheduled input from the operator surface if useful for the
   first-room gate rehearsal.

## Runtime Facts You May Need

- Deployed worker: built from determinism-hypervisor `ff1e88c` (includes the
  `5698d7e` framebuffer contract), UDS `/run/dh/grpc.sock`, 4 slots,
  snapstore per the rom-bridge-o73 runtime.
- Known operational caveat: bridge restarts orphan live slots
  (`rom-operator-bridge-72o`); if your acceptance runs share the deployed
  worker, coordinate timing with us rather than assuming free slots.

## Contact / Tracking

- Bridge-side tracking beads: `rom-operator-bridge-9z2` (closed — frame
  chain, contains the full diagnosis trail), `rom-operator-bridge-72o`
  (open — slot lease persistence).
- Cross-repo precedent for the request/handback pattern:
  `../determinism-hypervisor/.agents/requests/rom-bridge-getframebuffer-region-contract/`
  (00–06 tell the whole story, including the resolution and deployed
  verification format we would use here).
