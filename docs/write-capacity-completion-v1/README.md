# Completed write-capacity evidence

This packet records four completed local capacity stages. It reuses the original successful terminals, source/protected-binary readbacks, exact cleanup receipts and accepted retention records. It performs no new source test, workload, payload decode, restoration or cleanup.

| Stage | Actual tool terminal | Allocation/free-space result |
|---|---|---|
| Initial generated-cache cleanup | 88997 / b93a42 / 0 | 6,919 exact files removed; global available space increased 3,846,586,368 B to 18,603,966,464 B |
| Lossless tranche 040–055 | 87032 / 825382 / 0 | All 16 cohorts COLD; conservative net allocation recovery 7,291,015,168 B; 33,205,479,634 logical bytes checked by original whole-cohort readbacks |
| Post-development generated-cache cleanup | 33114 / e70b2e / 0 | 6,780 exact files removed; global available space increased 6,538,379,264 B to 25,446,240,256 B |
| Lossless tranche 056–060 | 86445 / d61c44 / 0 | All five cohorts COLD; conservative net allocation recovery 1,602,387,968 B; 7,990,723,083 logical bytes checked by original whole-cohort readbacks |

Logical readback bytes measure the preserved original data checked, not space recovered. Net retention recovery subtracts retained patch/toolchain/transaction/report allocations within the original accounting scope. Global available space includes concurrent host activity and later reporting. These measures are not interchangeable.

During 040–055, accounted free space rose from 17,988,681,728 to 18,915,549,184 B, a 926,867,456-byte increase, while the retained-data transaction recorded 7,291,015,168 B of net recovery. Parent development Cargo/proof work overlapped that interval and rebuilt caches. The 6,364,147,712-byte difference also includes reporting, timing boundaries and other host allocations; it is not an independently measured build-only byte count. Original controller status and subsequent report observations are retained separately.

The final 056–060 controller observed **27,046,117,376 B available** and stopped after 060, above its 26,851,934,208-byte release-plus-margin target. That was a **historical observation, not a reservation**. Root has since resumed builds. A later gate must refresh actual capacity. No 061 ran, no successful cohort was replayed, and older failed campaigns remain failed.

Both cache retirements retained exact source snapshots, protected executable hashes and the external-link exception, preserved Cargo fingerprints, and used build/package locks plus complete inode/link/reference checks. Post-dev cleanup bound clean main 5f78457277b6a0cdc7cde5a30067b2461d96a817 and candidate e2e23cca5e70a9ea0cc241877b3b35b5b6433d27. Source maps and before/after readback records are included; build/ELF payloads are excluded.

The retention tranches retain each selected cohort's original policy, original byte/hash/child predicates, exact reconstruction and corrected whole-reader acceptance. All 1,816 original objects, 41,196,202,717 logical bytes and 8,926 fresh successful codec receipts are represented by the existing accepted records. This packet does not replay or independently re-audit those payloads. Original complete local transaction roots remain the detailed byte/readback authorities.

The packet preserves metadata-only failures 8f7e1f/1 (first-tranche freeze permission) and 992253/1 (old completion-file read permission). Its own first selection attempt 609904/1 compared access time during a read; that reporting-only assertion was corrected to preserve device/inode/content-size/mode/ownership/mtime/ctime checks. The failed helper and failure receipt remain. Corrected selection passed 8806e5/0. None of these failures is relabeled as a successful runtime attempt.

`source-selection.json` lists every exact original source path and SHA. `metadata.tar.gz` contains those selected metadata files plus this report and its small preparation scripts. `archive-manifest.json` binds every member's original path, length and SHA; `readback.json` records independent complete gzip EOF and exact member-hash verification. Original WAL/data, retained compressed and patch objects, ELF/build payloads and large resource streams are not included and remain at their original locations. Archive member verification establishes portable metadata integrity, not a new benchmark or correctness acceptance.

This new reporting directory and its archive add separately measured metadata allocation. No original accounting or source root is rewritten. Packaging is finite: 4 MiB/member, 32 MiB file bytes, 40 MiB complete decoded tar stream, 8 MiB compressed, 64 MiB new directory allocation, 8 GiB current host floor and helper CPUs 6–15,22–31. The exact package and independent verification commands are the corresponding `sudo -n env PYTHONDONTWRITEBYTECODE=1 PYTHONOPTIMIZE=0 taskset -c 6-15,22-31 python3` invocations of `package.py` and `verify.py` in this directory. Completed outputs are never overwritten or automatically rerun.
