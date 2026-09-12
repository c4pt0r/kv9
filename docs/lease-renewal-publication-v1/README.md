# Lease renewal/publication source evidence

The [adapter report](../LEASE-RENEWAL-PUBLICATION.md) explains the implementation
and remaining clock, read-view and actual Chaos Mesh gates. No server lease read
path or lease performance result is established here.

- 231 library tests pass, zero failed/ignored; 12 are new (eight peer/driver,
  three wire, one controller). Default-feature restart, experimental compilation
  and warnings-denied Clippy pass.
- Eight compiled source faults fail exactly as intended; all eight target tests
  pass on baseline and restored source. All 36 control commands have their
  expected exit codes. These populations overlap the library tests.
- The initial wrong test import and the subsequent election fixture failure
  (230 pass, one fail) are preserved, along with sources and commands.
- Source and controls run locally under the unchanged CPU, cache-lock and disk
  reservations. No hosted CI or performance campaign ran.

The [archive](original-evidence.tar.gz) contains **251 exact files**, **691,934
bytes**, SHA-256
`242c85be67c43b317c6a2127646dd46cd165f9ee60fb0b506f8149c860905ae3`.
[Inventory](inventory.json), [source summary](source-summary.json) and
[control summary](controls-summary.json) bind the original sources, command logs,
source faults, first failures and maintenance/restore metadata. Every archive
member was checked byte-for-byte after writing. Full duplicate workspaces,
compiler caches and compressed executable payloads are excluded from this small
publication archive.

The accepted library test executable is retained locally at
`/tmp/kv9-lease-renewal-election-clock-source-first/raft-tests.gz`; its decoded
SHA-256 is
`0da7be3c916b5d3f18f018ed8b15d1f307c9e3372c3ef977d9c8a03f26ee450f`.
The source summary gives restoration instructions. The source supervisor exits
0 (`2ac909`); the control supervisor exits 0 (`2ce35c`). Original first-failure
supervisors exit 1 (`eb11df`, `b3af0f`). These are terminal command receipts, not
extra tests or a lease-enabled service acceptance claim.

To reproduce controls, run `python3 scripts/check-lease-renewal.py --output
/tmp/<new-owned-directory>` under the documented local source-build reservation.
The runner copies actual workspace/protobuf sources, uses the shared retained
build lock and never changes the caller's checkout.
