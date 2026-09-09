# Receive and replacement acceptance

The five original acceptance criteria of [#42](https://github.com/c4pt0r/kv9/issues/42)
are satisfied by the receive, store lifecycle, root formation and endpoint
recovery increments. This record reconciles their evidence against the original
scope. It does not close the broader C01–C04 or P0 roadmap gates.

The original failure at `38e76661dccb6f3db8375d6198c779dd16477b6e` remains a failed
run: a newly prepared replacement for promoted node 4 received a stale heartbeat
and panicked on `to_commit 40` with an empty log. A sender's root identity or an
unavailable replacement process was never sufficient evidence of safe rejection.

## Original acceptance matrix

| Original criterion | Implementation and protocol evidence | Executed acceptance |
| --- | --- | --- |
| Specify the receive/admission/recovery gate for initial voters, fresh learners, partial registration and Active restarts without circular catch-up. | [Receive authority](RAFT-RECEIVE-AUTHORITY.md) gates each incoming batch and the new dynamic owner's start until its own validated registration receipt. `RuntimeBackend::register` commits admission/incarnation before installing a versioned route and adding the learner. `local_member_is_active` requires the exact durable incarnation and ConfState. [Store lifecycle](STORE-LIFECYCLE.md) and [root formation](ROOT-FORMATION.md) cover initial voters and original-disk pre-catalog recovery. | Real registration, missing-init-marker restart, original-store formation crash cuts, missing-Active-log refusal and independently prepared replacement-disk tests; matching all-voter actual Chaos cells in the accepted matrix. |
| Prove safety and conditional progress with explicit assumptions, Rust synchronization mapping and invalid controls. | Parameterized receive, lifecycle and formation TLA+/TLAPS families map their transition boundaries to the runtime. [Route ownership](RAFT-ROUTING.md), [catalog CAS](ENDPOINT-MIGRATION.md), [writer ordering](ENDPOINT-WRITERS.md) and [endpoint recovery](ENDPOINT-RECOVERY.md) supply the later migration composition. | Fresh receive/store/formation checks: 41 declarations, 408 obligations, 90 TLC cases and 90 proof/audit cases, including 61 positive strict fresh-cache proofs. The accepted migration increment separately checks 57 declarations / 553 obligations across its four families. Isolated model, proof, semantic-audit and output faults are rejected. |
| Reproduce stale heartbeat/new disk with a real message, reject before RawNode effects, and retain wrong-ticket/incarnation and valid learner/restart controls. | `fresh_joiner_rejects_stale_heartbeat_before_raft_owner_starts`, `registration_refusals_preserve_routes_and_renewed_tickets_cannot_rebind`, and `a_real_joiner_follows_the_wire_hint_to_a_non_seed_leader_end_to_end` in [runtime.rs](../crates/server/src/runtime.rs) exercise the production gate and transport. | Five receive controls each pass baseline and restoration and fail their named assertion under one compiled mutation. Nine store controls cover identity, ownership, recovery-only opens, activation, formation ordering and failure cleanup. All 42 selected executions have exact one-test selectors. |
| Require a live replacement and positive typed rejection in root-trust E2E. | [root-trust-e2e.sh](../scripts/root-trust-e2e.sh) uses [local_admin_client.py](../scripts/local_admin_client.py) to require the current live child PID, empty fatal field, explicit invalid-ticket/incarnation rejection, closed receive/owner authority and commit zero. | The accepted executable root-trust path exercises invalid admission, live replacement refusal, original-store return, valid registration and restart. Seven current local-admin tests pass, including rejection/status evidence controls. Dead children and stale status files cannot establish rejection. |
| Pass exact-source executable MinIO, complete-history actual Chaos, persistence/proof gates and valid join/restart, without a new required DB singleton. | The accepted runtime source is `f73ee1fb1bfda37a531e9d9482a5cf46aa453580`; final acceptance fixtures are `f907da76b4aab7b4f899281c2b9fcb954d4c69bb`. [Endpoint recovery](ENDPOINT-RECOVERY.md#accepted-local-increment) binds the tested CLI binary and actual container bytes. Stable-endpoint restart uses durable local authority, and formation/recovery uses any available quorum. | 554 workspace tests pass, six executable E2Es pass, and real three-replica MinIO failover/checkpoint/reclamation/recovery passes. The complete actual Chaos Mesh matrix passes all 21 windows with independently checked histories of 4,024 CLI and 1,194 persistent-client operations. It includes every original voter's missing-log and replacement-PVC refusal, preformation recovery, Raft I/O faults and retained-PVC endpoint migration. The collector is removed before a final DB probe. |

## Closeout verification

This closeout changes documentation, the receive-control runner and one
`#[cfg(test)]` runtime function. It changes no production behavior. An independent
source comparison against `24618388de938b11b70c4a05c5c746461bb916b9` verifies that
the runtime is byte-identical outside that function and that every other tracked
non-documentation source, except the owned control runner, is unchanged. The
prior executable/Chaos evidence therefore remains bound to the production source;
these tests were not presented as a new full Chaos run.

The old early-route mutation targeted the removed unversioned `register_peer`
call and failed before testing. Its replacement injects a versioned route effect
before validation for a new node, preserving the legitimate later installation.
The invalid-ticket assertion detects that premature effect.

The renewed-ticket control now states its narrower result accurately. Removing
the outer typed incarnation refusal still reaches the independent endpoint
writer's binding guard, which returns
`Failed(Config("registration cannot replace an endpoint's store incarnation"))`.
The test first confirms the original route and Pending admission remain intact,
then checks the public `InvalidIncarnation` refusal. This mutation demonstrates
loss of the typed refusal, not actual rebinding; neither production guard was
weakened. The formal immutable-binding mutation remains a separate safety control.

Fresh local checks pass all 5 receive and 9 store controls, the three protocol
families above, 7 local-admin tests, denied-warning server Clippy, formatting,
Python syntax and diff validation. The independent audit rechecks copied/current
source hashes, one-source baseline/mutant/restored isolation, parsed TLC results,
strict proof obligation logs and semantic inventories. The original obsolete-anchor
failure and the intermediate pre-diagnostic-fix run remain retained separately.
No full workspace or Chaos rerun was needed for these test/documentation changes;
no GitHub workflow was dispatched.

The earlier production acceptance archive is
`target/correctness-evidence/2026-09-09-f907da7-endpoint-recovery.tar.gz`,
659,428,091 bytes, 18,351 independently verified entries, SHA-256
`8de800376319426c668aa8f9c6a3b4f02c8a95acbbef40ba7ed2d7b76ac83577`.
The closeout audit rehashes that archive and references it rather than duplicating
the payload. The test/control repair is committed at
`adf5ec8de255bf7a0a83340af1ea5d8b56d30863`. Its separate archive is
`target/correctness-evidence/2026-09-09-adf5ec8-replacement-acceptance.tar.gz`,
3,677,360 bytes, 1,520 independently verified files, SHA-256
`5d0a6c0313bf2df1a0ea3c8fe0e317b4195247bea42404bace7afe0ee6d1e272`.
It contains the committed source archive, documentation snapshot, all closeout
proof/control runs, failed/intermediate attempts and independent audit.

## Scope retained

The proofs establish the documented protocol projections, not a machine-checked
refinement of the whole Rust binary. They assume trusted membership/consensus
certification, unique independently generated store incarnations and the stated
filesystem durability contract. Their progress theorems require the documented
fair continuation and eventual available quorum. Real tests and isolated source
faults check the mapped implementation boundaries.

The Chaos evidence uses one Kind host. It does not establish cross-host failure
isolation, physical power-loss tolerance or throughput targets. Migration Chaos
uses a retained learner PVC; a separate real CLI test migrates a root voter.
Changed-address recovery requires an eventually reachable known peer or incoming
catch-up; stable-address recovery needs no live registration coordinator. The
supported writer transition is stop/upgrade/restart, with no mixed-generation
availability or downgrade claim.

Independently prepared replacement media and copied root descriptors are covered.
Bit-for-bit whole-disk clones, future numeric-ID reuse and general replica
replacement need their own ownership protocols under #24. These were not the
original #42 acceptance criteria. The initial gRPC apply/read-confirmation timeout
in #48 remains unexplained. Earlier failures in #43 and all failed migration
attempts remain failures, even where later unchanged histories were independently
accepted by the improved checker. Core range-history, bounded-storage and broader
P0 work remains tracked under #9.
