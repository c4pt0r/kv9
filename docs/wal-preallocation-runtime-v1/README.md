# WAL preallocation runtime evidence

Start with the [report](../WAL-PREALLOCATION-RUNTIME.md), [result](result.json),
[build readback](build-readback.json) and [recovery audit](recovery-audit.json).
This packet records matching default/feature release builds and actual ordinary
three-voter recovery. It is not a performance or actual Chaos Mesh result.

`evidence.tar.gz` contains 239 original members, 13,278,536 logical bytes and
2,035,841 compressed bytes; SHA-256:
`99f93ecbde7fcc6cc3c1c227b64c9517121f64758737537290bf045f68328b18`.
Every regular member is byte-checked against its retained original and
`archive-inventory.json`. No extraction is needed to verify the archive.

- `build/` and `correctness-client/` retain original source inventories, Cargo
  messages, compiler logs, commands, cache authority and separate build records.
  The matching releases and later native client have distinct producing commands.
- `recovery-input-*` contains the explicitly composed catalogs. The original
  server/client manifests remain in their original directories. Large executable
  copies are excluded and listed with exact local paths/hashes in `result.json`.
- `recovery-default/` and `recovery-preallocated/` contain all four histories,
  witnesses, reports, final drains, process lifetimes and fixture logs. Their
  `retained-wal/` directories contain all 66 completed fixture files. The original
  NVMe paths remain intact and independently verified.
- `recovery_binding.py` and `normal-build-original.py` show the narrow feature
  extension. `binding-controls/` retains each altered input catalog; the result
  records two accepted inputs and five rejected controls. Executables remain
  local and are never edited by these controls.
- `run-recovery.py` preserves the actual storage/feature adapter. Its scenario
  and history checker are the unchanged scripts in source revision `86aa6fc`.
  `audit-recovery.py` independently checks original histories, witness coverage,
  final reads, progress windows, fresh drains, lifetimes and all retained bytes.
- `source-preflight-first.json` preserves the original pre-Cargo failure;
  `source-inventory-tests.*` contains all eleven successful inventory regressions.
  `source-equivalence.json` binds unchanged Rust/Cargo/proof inputs to `fb9d390`.
- `actual-tool-receipts.json` records real terminal handles. The archive assembly
  and its full member readback completed separately as `9066bb/0`.

All original output remains under
`/mnt/data/kv9-work/wal-preallocation-runtime-20260915-first`. Reproducing the
original helper commands requires their pinned checkout and retained binaries;
this archive does not claim to contain either the entire checkout or ELF files.
The separate [source/proof packet](../wal-payload-preallocation-v1/README.md)
retains prior strict checks and microbenchmarks. No private kubeconfig is included.
