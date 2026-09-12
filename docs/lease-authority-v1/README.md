# Lease authority transition validation

See the [transition contract](../LEASE-AUTHORITY-MODEL.md) and
[clock/history proof](../LEADER-LEASE-PROOF.md). Production remains selected
`11113f6` / Safe ReadIndex. This stage runs no Rust candidate, benchmark,
hosted CI, clock-platform qualification or actual lease Chaos Mesh campaign.

The final local runner completed successfully:

- **27 TLAPS theorems / 348 obligations**, including initialization, all 18
  action cases, temporal induction, local read steps and successful-read authority.
  Baseline and restored checks use fresh caches with `--strict --nofp`.
- SANY checks exact theorem/import inventory, the single legal-input assumption,
  pinned standard modules and absence of proof holes. Added-axiom and omitted-proof
  controls are rejected; three incomplete-output controls are rejected.
- Complete three-voter finite exploration: **56,919,883 generated / 9,674,978
  distinct states / zero queued**, depth 29. The arbitrary-time/round/node safety
  claim comes from the deductive theorem, not the finite enumeration.
- Six faulty protocol variants produce named TLC invariant violations and
  execution traces: lost recovery quarantine, prior-vote grant acceptance,
  voting during a promise, publication without quorum, stale read frontier and
  reading before apply.
- Six unconstrained reachability witnesses show a successful read, replacement
  write, expired certificate, restart with historical promise, apply lag and
  an old generation. A seventh witness checks an explicit 16-action sequence
  that acquires a lease, performs a read using only local steps, expires the
  first certificate and acquires the next round. Every scheduled action also
  satisfies the unchanged bounded protocol relation.

An expired lease cannot satisfy the successful-read theorem's strict deadline.
The fast-path fallback has no transition to success. A production request must
obtain a new valid lease or fresh Safe ReadIndex authority; failure to obtain
quorum confirmation means typed unavailability or its original timeout. This
does not weaken the clock/voting/recovery premises while a lease appears live.

The [original evidence archive](original-evidence.tar.gz) contains **255
files / 161518 bytes**, SHA-256 `3b8f4032321bd5d6c2e87c9ce170c88fbb8e476ac768715b5ce6449581e10016`. Every member was read back and hashed;
[inventory.json](inventory.json) binds their names, sizes and hashes. The original
[summary.json](summary.json) binds commands, verdicts, source and toolchain hashes.
TLC state stores and TLAPS caches are excluded from publication; retained
sources, semantic audits, unabridged logs, traces and result records are included.

The archive preserves these unsuccessful development checks:

- Proof drafts 1–4 retained unresolved automatic quorum/recovery obligations.
  Explicit quorum-intersection decomposition completes draft 5; draft 6 adds
  the local-read and finish-authority theorems. SANY then rejects a shadowed
  bound variable that TLAPM had accepted. The final source renames that variable
  and passes both tools; no safety premise or protocol guard is removed.
- The original full model and its vote-timestamp projection each time out at
  180 seconds. The complete symmetric configuration preserves the original
  time/commit/round/read bounds and only identifies the two indistinguishable
  followers. The earlier time-zero run is separately scoped.
- The first combined campaign passes its proof, full finite safety configuration,
  all six fault controls and six witnesses, but the unconstrained two-round
  reachability search times out. That campaign exits 1 and remains incomplete.
  The final runner uses the explicit legal sequence for renewal reachability;
  it does not claim completion of the failed broad two-round search.

The archive also includes a read-only Chaos fixture review. Existing Safe
ReadIndex partition/delay/restart machinery is reusable preparation, not lease
acceptance. Selective renewal-message faults, asymmetric voter links, live
serving-process pauses and clock-compatible observations still need concrete
implementation and actual fault histories.

Reproduce the final suite with a new output directory:

```sh
taskset -c 6-15,22-31 env PYTHONDONTWRITEBYTECODE=1 PYTHONOPTIMIZE=0 \
  python3 scripts/check-lease-authority.py \
  --tlapm /tmp/kv9-p0-tools/tlapm-1.6.0-pre-20260731/bin/tlapm \
  --jar /tmp/kv9-p0-tools/tla2tools-v1.7.4.jar \
  --output /tmp/kv9-lease-authority-new
```

TLAPM is `4600b24`; the pinned TLC/SANY distribution is `1.7.4` (TLC identifies
itself as `2.19`, revision `5a47802`). Each invocation retains the 180-second
limit. TLC uses one worker, fingerprint 0 and a 512-MiB Java heap. The final
suite is a fixed-configuration protocol proof and bounded validation, not a
complete Raft implementation/refinement or E2E proof.
