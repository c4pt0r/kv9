# Endpoint migration and retained-store recovery

Tracking: [#47](https://github.com/c4pt0r/kv9/issues/47), under
[#9](https://github.com/c4pt0r/kv9/issues/9). This extends the
[catalog CAS](ENDPOINT-MIGRATION.md), [writer ordering](ENDPOINT-WRITERS.md)
and [transport ownership](RAFT-ROUTING.md) contracts. The implementation and
acceptance fixtures are present. The accepted local increment below closes #47's
production endpoint-migration scope; the broader #42 and P0 gates remain open.

## Operator workflow

1. Upgrade the supported writers at a planned maintenance boundary, as described
   below. Keep the original data directory/PVC, root descriptor and incarnation.
2. Read the current endpoint from a metadata leader with
   `kv9 client get-node-endpoint --addr LEADER --node-id NODE`.
3. Stop the original owner. Configure its successor with the original data
   directory and `--advertise-addr NEW_ADDRESS`. `--addr` is independently the
   listener bind; wildcard binds and Service/NAT advertisements remain supported.
4. Submit `kv9 client change-node-endpoint --addr LEADER --cluster-id CLUSTER
   --node-id NODE --store-incarnation INCARNATION --expected-address OLD_ADDRESS
   --expected-generation GENERATION --new-address NEW_ADDRESS`.
5. Wait for the successor's `endpoint_ready=true`, `bootstrap_state=Serving`
   and a fresh `endpoint_confirmation_term/index` covered by its applied prefix.
   A retained store can pump Raft before its public Serving gate opens.

The update may precede process startup. Both endpoint RPCs use the existing
bearer authentication and bounded metadata admission. The CLI sends one request;
transport failure, timeout or an incomplete/contradictory response remains
`endpoint_outcome=unconfirmed`. It never silently retries an ambiguous update.
An explicit identical CAS retry can return a distinct `confirmed` result with
a new `confirmation_term/index`. It does not reconstruct the original mutation
receipt. A stale or foreign precondition returns a typed refusal without
changing endpoint state, root identity, incarnation or Raft membership.

## Authority and recovery

The public update acquires the catalog planner mutex, drains ambiguous earlier
commands through an ordered barrier, evaluates the full CAS against one fresh
snapshot and proposes under the same leader term. A successful change is one
committed batch. An exact duplicate appends a fresh `Command::Noop`. Only an
exact applied receipt permits an eager route installation and a success response.

Credential cancellation is distinct from decommission. Endpoint changes mark
Pending/Consumed admissions **Superseded** (wire value 4) in that same batch.
Neither a superseded ticket nor its consumed retry can admit a replacement store.
An already registered member retains authenticated node traffic. A superseded
credential without its member grants no traffic or receipt-window fallback.
Explicit **Revoked** state continues to deny member authentication, and endpoint
updates cannot lift it. A new authenticated admission is a separate operation.

On a changed-address restart, the Active same-incarnation store retains its
durable Raft receive/owner authority. Public data service begins closed. It tries
root seeds, locally applied Active catalog entries and the local control socket
under a rotated, deduplicated, bounded discovery budget. Hints are routing
candidates. `ConfirmEndpoint` independently checks the authenticated subject,
root digest, cluster, Active membership, incarnation and current advertised
address, then commits a fresh term-fenced noop. The response names both the
subject route and responding leader route at that exact position. It grants no
new membership or address mutation and requires no obsolete join ticket.

A remotely confirmed leader route can be newer than the recovering node's local
catalog. Every installed catalog route therefore carries its generation. Older
catalog snapshots and unversioned bootstrap fallbacks cannot replace it. Equal
generations with different addresses are errors. Advancing a version at the same
address preserves the existing connection ownership. Existing immutable transport
destinations still govern send/update races and retired worker cancellation.

The recovering owner waits for the exact confirmation's local application. It
then checks a fresh local Active endpoint row for the same incarnation, address
and confirmation generation. A superseded or unconfirmed receipt grants no
Serving authority. A bounded expired wait may request another fresh noop; it
never substitutes a watermark for the missing receipt of that attempt.

Before opening public Serving, the owner publishes `kv9-serving-endpoint`:
`KV9ENDP2`, strict bounded JSON and SHA-256. The record binds root, node,
incarnation, advertisement, generation and exact confirmation position.
Publication writes a unique temporary file, fsyncs it, renames it and fsyncs the
directory ancestry. Every failure returns before in-memory authority is granted.
Reopening a visible record validates and stabilizes it before use. Truncation,
checksum errors, foreign bindings and invalid receipt positions fail closed.

A stable-address restart can use that saved authority, a matching local Active
catalog endpoint and durable applied coverage without contacting a coordinator.
The check uses the persisted command position: the process-local unified driver
watermark intentionally starts empty after restart. Generation-zero initial and
registration authority can establish the first marker; an absent marker at a
migrated generation needs a new confirmation. Serving is not a promise of
immediate global endpoint revocation: an already running node observes later
directory changes through normal application/runtime polling, while all data
operations retain their existing consensus/read gates.

## Supported writer floor and rollout argument

The minimum legacy writer exercised by the upgrade fixture is
`bc5bbd2e8c73c4e811b720f17ab74b0ff7e714f4`. The new lifecycle reader accepts
`KV9LIFE1` solely for an in-place upgrade; every new publication uses `KV9LIFE2`.
Before opening Raft, the new binary republishes an existing V1 lifecycle record
as V2 under the exclusive `StoreGuard`, preserving every node/incarnation/phase/
root field. Failure cannot start an owner. The immutable root and store identity
files are unchanged. Independently prepared replacement volumes remain refused.

The format argument assumes the existing exclusive store-lock and filesystem
durability contracts: a successful file/ancestor fsync survives a process crash,
and a physical store is not concurrently accessed through aliases bypassing its
guard. While a new owner holds the guard, an old owner cannot obtain it. After
successful V2 publication and guard release, the supported old decoder rejects
the V2 header before it can open Raft. If publication fails before durability,
no new owner was granted authority; reopening validates/stabilizes the visible
record or repeats the upgrade. Therefore no supported old owner can resume a
store after the new writer has received its start authority. Arbitrarily older
binaries predating the store-guard contract are not a supported downgrade path.

The internal service is now `kv9.raft.v2.Kv9Raft`; the public service remains
`kv9.v1.Kv9`. The old and new internal service paths are disjoint. Neither server
registers the other path, so cross-generation Raft, discovery and registration
calls fail before backend execution. Combined with the durable writer floor,
an older registration writer cannot participate in a new-generation quorum and
silently overwrite endpoint versions. This composes with existing Raft quorum,
term and durable-prefix guarantees; it does not introduce another coordinator.
The real-binary wire probe tests both directions, with matching-path
authentication refusal as a live-server control.

The supported rollout is a planned stop/upgrade/restart of metadata writers.
Mixed-generation availability and downgrade to a pre-floor binary are not
supported. Keep client routing configuration current separately from internal
advertisement. A changed-address recovery requires a reachable known peer or
eventual incoming catch-up that updates its catalog; exhausted stale candidates
do not manufacture discovery authority. Recovery has no required singleton
seed, operator process or collector. Object storage retains its existing
external dependency boundary.

## Proofs and acceptance boundaries

`EndpointRecovery` ranges over arbitrary finite ordered histories, addresses and
generation assignments, with arbitrary operator changes, delayed local apply,
receipt loss/expiry and restarts. Its 15 TLAPS declarations / 165 obligations
prove inductive authority, fresh confirmation positions, publication only after
application, guarded admission and conditional recovery progress. The progress
premise explicitly includes a stable authorized endpoint, an available fresh
noop, eventual successful confirmation/application/publication and no further
crashes or expiration of that successful attempt. Permanent partitions or
endless superseding updates do not satisfy it. `wait_applied` and durable
publication are refinement boundaries with separate implementation controls;
this is not a proof of the entire Rust binary or Byzantine peers.

The extended `EndpointWriters` proof has 14 declarations / 111 obligations. It
allows a remotely certified installation during an older snapshot, proves the
monotonic generation floor and convergence after directory stabilization, and
separately preserves member decommission state while cancelling obsolete join
credentials. The former mutex-only rollback control is retired because the
generation guard now independently prevents that fault.

Local gates include 17 compiled source mutations; full fresh-cache proof and
model checks with ten recovery/eight writer protocol mutations; malformed
receipt and durable-publication regressions; and real CLI upgrade/migration
acceptance. The stronger paused-apply test first applies the address CAS, then
checks the Serving gate after every runtime step while the *fresh confirmation*
is unapplied. This catches even a transient early-Serving window.

The actual Chaos extension kills the registered learner with PodChaos, retains
its original PVC, confirms owner-lock release, starts a distinct advertised
Service endpoint and proves the old Pod endpoint unavailable. Both complete
client histories must show work during the pending and recovered intervals;
the new learner must apply a fresh receipt and return its normal typed leader
refusal through the new socket. The existing voter-kill, partition, I/O,
missing-log and independent replacement-PVC cells remain required. Kind runs
on one host; this is not multi-host availability or a hardware power-loss test.

Daily gates run locally. Hosted workflows remain manual-only for pre-release
or key-milestone acceptance after local gates pass.

The first full matrix attempt at `f73ee1f` stopped during voter 3 I/O recovery,
before the new migration cell. The database had exited through the expected
Raft `EIO` path. After IOChaos deletion, the idle Bash fixture supervisor was
still in `T (stopped)` with `TracerPid=0`; no database child had been launched.
The retained daemon trace shows toda's attach/detach sequence during healing.
Sending `SIGCONT` to that launcher immediately started the original store,
which recovered Serving and caught up. The failed run remains failed.
The fixture now records supervisor state and explicitly heals an untraced,
stopped launcher only after the database exit and FUSE unmount are established.
It retains the original I/O failure, majority history, fresh-process, exact
catch-up and recovery-metrics requirements.

The second full attempt passed every existing matrix cell and stopped before
the migration update because the new learner Pod appeared before the old
process released its store lock. A later same-store `store-prepare` succeeded
without modifying its identity. The migration fixture now waits for actual
lock acquisition and retains every attempt; it does not infer owner death from
Pod UID replacement. This preserves the exclusive-owner requirement.

The third attempt exposed a second launcher race during fault installation:
toda left the polling `sleep` child stopped while Bash waited for it. No new
database process existed. Resuming that child launched the original store and
immediately produced the required Raft `ENOSPC` fatal exit under the still-live
IOChaos mount. The fixture now waits with a Bash builtin on an idle FIFO,
without spawning polling children. Before each injected/healed start it records
the launcher state, verifies no child/tracer and the expected FUSE mount state,
and clears pending stop signals on the launcher. Database failures, in-fault
histories and recovery remain separate mandatory observations.

The fourth full attempt completed all 21 fault windows and the actual migration
cell. The complete persistent-client history passed, but the CLI history checker
exhausted its frontier and correctly returned `inconclusive`. [#49](https://github.com/c4pt0r/kv9/issues/49)
tracks the confirmed-range snapshot ordering repair. The original
4,330-operation history is unchanged; its full witness is accepted by the original model.
The [history contract](HISTORY-CHECKING.md#confirmed-range-snapshot-ordering)
records the permutation argument, reduced failure and unchanged budgets.

The fifth full attempt also completed all 21 fault windows, while its complete
4,280-operation CLI history exhausted the search budget. It requires a range
to capture an overlapping insertion, the opposite priority from the fourth
history. The existing two guided attempts now prefer different snapshot orders;
both unchanged histories yield complete replayable witnesses within the original
limits. Both original command failures remain retained. The seventh full matrix
supplies the fresh acceptance recorded below.

The sixth full matrix completed every fault window and its persistent history,
but its 4,264-operation CLI history again exhausted the checker budget. Reduced
histories expose mixed snapshot order and an unknown write needed by a read
overlapping a confirmed mutation. The [history search contract](HISTORY-CHECKING.md#overlapping-observations-and-mixed-snapshot-order)
records the repair and its preservation argument. This failed command remains
failed; the seventh complete matrix supplies the acceptance recorded below.

## Accepted local increment

The runtime and proof sources are frozen at `f73ee1fb1bfda37a531e9d9482a5cf46aa453580`.
The final fixture/checker revision is `f907da76b4aab7b4f899281c2b9fcb954d4c69bb`.
Independent source audit verifies that later changes are confined to the two Chaos fixtures,
history search guidance and its controls, manual workflow counts, and
documentation. The independent AST audit confines checker changes to read hints,
candidate priority and expanded guided targets. The complete model, witness
verifier, budgets and verdict logic are unchanged.

- **554 workspace tests passed, zero failed, 22 ignored.** Formatting,
  warnings-denied Clippy, fresh rustdoc for all eight workspace packages,
  Python/Bash syntax and workflow validation pass. Ignored tests are not counted
  as passes.
- All six executable E2E paths and real MinIO failover/checkpoint/recovery
  passed. Production CLI acceptance covers pre-upgrade acknowledged data,
  V1-to-V2 recovery, both disjoint internal wire directions, old-writer refusal
  without store mutation, public CAS/fresh duplicate confirmation, root-voter
  migration and stable restart with every other database process stopped.
- Seventeen compiled source controls pass baseline, intended failure and
  restoration. The recovery/writer proofs establish **15/165** and **14/111**
  declarations/obligations; CAS **11/80** and transport ownership **17/197**
  regression gates also pass. Independent source binding accepts 134 TLC cases,
  130 proof/audit cases and 88 positive fresh-cache proof runs across these four
  inventories, with their rejecting protocol, semantic and output controls.
- All **21 actual Chaos Mesh windows** pass. The complete CLI history has
  **4,024 operations**; the persistent-client history has
  **1,194 operations**. Both original histories are
  replayed in an independent copy. Every-voter I/O, missing-log and replacement-
  PVC evidence, formation, collector removal and all five corrupt migration
  evidence controls pass. The actual learner move is from
  `10.244.0.193:20160` to `10.96.212.155:20160`,
  with mutation index **2377** and fresh locally
  applied confirmation index **2380**.
- The history checker passes **44 tests / 21 source mutations**. All complete
  histories from the fourth, fifth and sixth attempts also pass independent
  replay. They contain 4,330 / 4,280 / 4,264 CLI operations and 1,238 / 1,236 /
  1,201 persistent-client operations, respectively. Their original inconclusive
  command results remain retained, alongside the three earlier fixture failures. The unchanged
  certificate/interval foundation also passes the complete 24-theorem Lean
  inventory and all nine invalid proof controls.

The accepted production executable is SHA-256
`10045686b6683acc4c5f83cfb3d46850a5ee5682302d5c5e6abadffa1f11d31d`.
The exact same bytes passed the real CLI acceptance and run inside image
`sha256:3a54bc77189a8fcd89b91dcccdc630bfccc3211b41d2c87517725a5619fc2a9c`.
The build manifest binds the final clean revision and workload executable separately.

Archive: `target/correctness-evidence/2026-09-09-f907da7-endpoint-recovery.tar.gz`,
**659,428,091 bytes**, SHA-256
`8de800376319426c668aa8f9c6a3b4f02c8a95acbbef40ba7ed2d7b76ac83577`.
All **18,351 entries** were independently read and hashed from the completed
archive. It retains source snapshots,
executables, proof attempts, source controls, all failed runs, both accepted
complete histories, raw fault observations, diagnostics and independent audits.

This is single-host Kind evidence. Changed-address recovery still requires
an eventually reachable known peer or incoming catch-up. The supported writer
upgrade is stop/upgrade/restart; mixed-generation availability and downgrade
remain unsupported. The separate initial gRPC apply/read-confirmation timeout
in #48, #42's broader replacement boundary and C01–C04 acceptance remain open.
No GitHub workflow was dispatched for this increment.
