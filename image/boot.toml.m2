# The M2 acceptance image's boot manifest (API.md §7): autostart the trivial
# workload with an EMPTY expected-regions list — Ready fires once the unit is
# exec'd, and its doorbell-exit icount is the bit-reproducibility gate.
# Staged into the initramfs as /etc/detguest/boot.toml by tests/vm.
boot_toml_version = 1

[autostart]
unit = 0

[[unit]]
id = 0
exec = "/opt/autostart-trivial"
log_mask = 0x1F

# Unit 1 is started via ring-C StartWorkload for the LogLine/WorkloadExited
# acceptance leg (exits with code 7 after fixed stdout/stderr lines).
[[unit]]
id = 1
exec = "/opt/print-lines"
log_mask = 0x1F
