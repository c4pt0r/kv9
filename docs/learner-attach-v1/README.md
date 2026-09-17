# Learner-attach validation packet

See [implementation, attach semantics and remaining scope](../LEARNER-ATTACH.md).

- 917 passing workspace tests/doctests in the serial qualifying run, 34
  ignored, zero failures; strict Clippy/formatting pass.
- The runtime test covers attach refusals (uncommitted operation), idempotent
  confirmation with a freshly advanced cut per receipt, and the destination
  entering the group configuration as a learner and never a voter.
- Nine checked Lean theorems for the attach model with seven semantic
  mutation controls and two proof-policy controls; nine sibling Lean models
  and the retention TLA/TLAPS model re-accepted against the exact working
  tree.
- One accepted four-process e2e (`e2e-third`) chaining the full migration
  flow to the edge of runtime startup: committed intent → attach (learner
  committed through the group's own log, cut advanced past the configuration
  entry) → plan/bind/capture with the image naming the learner → **offline
  installation at the destination's real stopped store** via the new
  `install-migration-image` command, with the exact captured cut in the
  receipt and `serving=false` → destination restart, healthy, with the
  installed generation isolated by RegionManager. A control image whose
  configuration lacks the destination is refused by the unchanged membership
  gate at the same store; the same operation refuses a second image after
  the attach advanced the cut. Two failed e2e launches are retained: both
  were local-application races of just-committed rows, resolved by bounded
  read-only/idempotent client retries — the safe-direction refusals working
  as designed.

[evidence.tar.gz](evidence.tar.gz) contains **763 readback-verified files**,
768829 compressed bytes and 4510139 decoded bytes. SHA-256:
`80cf5960945b33e454ce85b75420f9aae9409b24b9468066a10e0043fb4cbbb8`.
[validation.json](validation.json) inventories all members. A separate
extraction re-verified every member, the e2e/proof receipts and the offline
install receipt; [portable-readback.json](portable-readback.json) records
that result.

Tested source is based on `3bfa47dfda89d784fc7f1bf213a8baa7b84d736b` with
the exact current Rust/Cargo/proto source retained under `source/`.

Nothing here promotes a voter, replicates toward the learner, starts the
installed generation, serves from it, records destination-install evidence,
quiesces or releases a pin, or claims catchup, split, placement, scaling or
new QPS. This increment ran no Chaos Mesh campaign. The independent
3/6/9-host benchmark contract in
[HORIZONTAL-SCALING-PLAN.md](../HORIZONTAL-SCALING-PLAN.md) remains the
acceptance criterion for effective horizontal scaling. No hosted CI was
dispatched.
