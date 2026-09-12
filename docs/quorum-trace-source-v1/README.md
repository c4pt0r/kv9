# Quorum trace source qualification

The first root-owned local release-profile qualification completed with exit 0
(session 23090, receipt d0261f). Default workspace: 709 passed / 23 existing
ignored. Quorum trace: 453 passed / 1 existing ignored. Existing read-stage:
438 passed / 1 existing ignored. Fifteen new focused trace controls were also
run separately. These are overlapping test populations. Formatting and all
three explicit Clippy configurations passed. No command failed or was rerun.

`source/result.json` records the exact commands. `sources-before.json` and
`sources-after.json` bind the tested working draft. `cache-safety.json` records
78 artifact observations, including 59 first observations after first-party
invalidation. `root/source-result.json` retains the 80 GiB minimum filesystem
floor and 16 GiB additional reservation; the lowest observed available space
was 111,638,323,200 bytes. `root/compiled-input-readback.json` independently
compares all 149 Rust/Cargo/protobuf input hashes after documentation changes.

`inventory.json` binds every other file by relative path, byte count and SHA256.
The source snapshot predates this evidence directory, later documentation and
reader additions, so it does not recursively identify its own publication.
Compiled inputs remain identical. This is source qualification, not a server
release, timing, recovery, Chaos Mesh or core Raft proof result. No hosted CI
was dispatched.
