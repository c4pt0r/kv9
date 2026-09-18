# Migration-abort validation packet

See [the wedge, the settlement algebra and limits](../MIGRATION-ABORT.md).

- 939 passing workspace tests/doctests in the serial qualifying run, 35
  ignored, zero failures; strict Clippy/formatting pass; 8 real-MinIO
  focused installer tests. New catalog tests cover idempotent abort
  planning, both directions of the evidence/abort exclusion, readback
  rebinding refusals, the view-level settlement probe, and the
  re-migration unlock (whole readback with aborted history committed; a
  live successor still blocks a third).
- Thirteen checked Lean theorems for the migration-abort model with
  five semantic mutation controls and two proof-policy controls;
  twenty-six sibling Lean models and the retention TLA/TLAPS model
  re-accepted against the exact working tree.
- Three accepted five-process e2e runs (`e2e-eighth` covers the exact
  final source) and FIVE retained failures, each a real finding that
  produced a fix: the Consumed-admission wall (the `revoke-admission`
  operator verb now exists), the immutable-incarnation wall (a fresh
  PENDING admission now authorizes the identity takeover at
  registration), two stale-heartbeat panics at the re-provisioned
  empty-log peer (the raft step layer now drops heartbeats whose commit
  floor exceeds the local log end), and an attach replication-lag flake
  (bounded idempotent retry). The failure logs carry the exact
  diagnosis lines, re-verified by the portable readback.
- The accepted run: a REAL stranding (attached, image published and
  captured, destination killed before install) wedges quiesce and
  release; the committed abort (kind 108, mutation then idempotent
  confirmation) settles the ledger through the UNCHANGED verbs
  (Published→Quiesced→Released, destination pin never dropping); attach
  refuses the aborted operation permanently; the stranded learner
  detaches at the source leader; and a FRESH incarnation re-provisioned
  onto the same node id completes a new migration of the SAME region
  end to end — install, adoption, retained-tail catchup, evidence,
  quiesce, release.

[evidence.tar.gz](evidence.tar.gz) contains **1744 readback-verified
files**, 1477755 compressed bytes and 8901927 decoded bytes. SHA-256:
`bdb69b6f2a3d284b8c8407535cb308ed0b54532de20de9dd643637470b0089cf`.
[validation.json](validation.json) inventories all members. A separate
extraction re-verified every member and the e2e/proof/retention
receipts; [portable-readback.json](portable-readback.json) records that
result.

Tested source is based on `2ec7df1` with the exact current
Rust/Cargo/proto source retained under `source/`.

Nothing here truncates the aborted operation's source log (the
truncation decision still requires committed evidence), reclaims the
abandoned image from the object store, aborts automatically, or takes
over voter identities (refused by design). No Chaos Mesh campaign ran.
The independent 3/6/9-host benchmark contract in
[HORIZONTAL-SCALING-PLAN.md](../HORIZONTAL-SCALING-PLAN.md) remains the
acceptance criterion for effective horizontal scaling. No hosted CI was
dispatched.
