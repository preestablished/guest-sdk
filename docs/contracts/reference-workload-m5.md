# Reference-workload M5 compatibility contract

guest-sdk consumes the reference-workload handoff; it does not own emulator,
ROM, feature-map, or full-suite behavior. The receiving-side contract is:

| Surface | Required contract | guest-sdk proof |
|---|---|---|
| Image identity | `reference_workload_git_rev` `7b0c7b2434e71d8b3241bf78597be457b281292d`; manifest BLAKE3 `af14040444db6f5e182f52193d71abdbbfb8085673b45da76c21dc541ac3dceb` | CI `Prepare evidence and pinned real image` rejects missing files, source/stamp drift, or image/initramfs/kernel digest drift before cargo. |
| Control | fd 3, `refwork-ctl` proto v1; `Hello → LoadGame → Start`; agent withholds Ready until Start and region publication succeed | `refwork_ready_hold::real_harness_reaches_and_holds_ready`; synthetic protocol pin in `m9_refwork_contract`; agent control tests. |
| Regions | live `wram`, `framebuffer`, `meta`, each layout version 1; framebuffer is 229,376-byte XRGB8888 256×224 stride 1024 | real-image Ready reports region count 3/generation 6; `m4_acceptance` resolves and reads all named extents across restored children. Optional `vram` is not required by the pinned image. |
| Input | workload reads pv-pad held latches; SDK emits FrameMark before FRAME_COUNTER; input data never enters control ring I | `decoded_pad_sets_land_at_frame_and_match_once_per_frame_polls` covers multi-port, same-frame latest-wins, sparse/held values, exact poll logs, ordering, replay, and negatives. DHILOG decoding remains hypervisor-owned. |
| Inject decisions | stable named SDK points emit InjectQuery before OUT; exactly one matching PIO answer; workload echoes canonical `ms5.inject.v1`; replay consumes decoded decisions with no synthesizer | `m5_inject_roundtrip`; `determinism_replay`; 1000-run summary under the execution request evidence directory. |
| Full suite | 20/20 double-run plus restore continuity, zero flakes; deliberate nondeterminism localizes divergence | upstream report BLAKE3 `a06051df0ce076daa49f48298b25959b7a83dac8deb23cf247177f6c2bbe13c3`, validated in `.agents/requests/phase3-ms5-execution-in-vm-closeout/05-prep-notes.md`. |

Artifact roles are intentionally separate. The reference image proves the
real workload lifecycle and compatibility surfaces at its pinned guest-sdk
revision. The guest-sdk-built `testload`/`m4-regions` images prove newly added
inject, decoded-input, and record/replay paths at the current guest-sdk SHA.
No claim that the pinned reference image contains newer call sites is made
without rebuilding and inspecting that image.

DHILOG bytes and VerifyReplay gRPC are owned by determinism-hypervisor.
guest-sdk accepts neutral decoded PAD_SET/decision values and records the
external VerifyReplay evidence reference; it does not implement another log
codec or worker client.
