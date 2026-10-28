# kv9 development roadmap

> **Range split moved out of master (2026-10-28).** Every split increment recorded below
> (split intents, parent seal, child population, publication, auto/cascade split, TTL split
> state/format/runtime/maintenance) now lives on the `experimental/range-split` branch; the
> linked documents, proofs and scripts were moved with it. `master` carries no split protocol,
> retires Raft tags 11/12/13 and task kind 107, and keeps the range directory and epoch fence.
> Split and merge remain design goals (DESIGN §10); their re-entry starts from that branch.

> **Production scope (2026-09-26): Raw KV, TTL, range split and range merge.**
> All four are required for the first production release. Close the shared
> storage, backpressure, recovery and resource-lifecycle gaps; preserve TTL
> through split/merge and recovery; qualify the features together before
> performance promotion. See the [scope and acceptance contract](RAW-KV-PRODUCTION-SCOPE.md)
> and [production review](PRODUCTION-READINESS-REVIEW-2026-09-26.md).
> Existing proof, actual Chaos Mesh and cross-host scaling gates remain.

> **TTL split state progress (2026-10-04):** [Durable transfer state](RAW-TTL-SPLIT-STATE.md)
> preserves original expiry and inherited watermarks, resumes cursor-checked pages after
> three-replica WAL reopen, and validates atomic transfer batches during recovery. Ordinary
> proposals remain disabled pending the all-member format gate, source authority and runtime
> copy/publication integration. Incomplete imports remain WAL-backed; production scope stays open.
> The subsequent [V5 store and service fence](RAW-TTL-SPLIT-FORMAT.md) now excludes V4 owners,
> paths and receipts; all-member TTL activation requires V5. Transfer submission and source
> authority remain disconnected, and uninterrupted rolling upgrades are not qualified.
> The subsequent [TTL split runtime integration](RAW-TTL-SPLIT-RUNTIME.md) now connects applied
> catalog plans, real sealed source snapshots, all-member confirmation for each transfer command,
> copy resumption and verified directory publication. A three-replica child resumes after WAL
> reopen on a new leader. Staging checkpoints, large rows, bounded scheduling and full deployment
> acceptance remain open; this does not complete the production scope.
> The subsequent [split maintenance worker](RAW-TTL-SPLIT-MAINTENANCE.md) separates new triggers
> from committed-intent resumption: threshold zero no longer freezes committed work. One shared
> heavy job per node drives seal/populate/publish outside the control loop and drains before
> store release. Three-replica automatic completion and admission tests pass. Catalog discovery
> is still unpaged; staging checkpoints, WAL reclamation and full deployment gates remain open.
> **2026-10-05:** [Staging checkpoints](RAW-TTL-STAGING-CHECKPOINT.md) now preserve unbound import
> state through actual publication, worker settlement and recovery. Three replicas select an image
> before Finish, reclaim engine WAL from 18.5 MB to 2.7 MB, reopen and finish on another leader.
> Dedicated tag 13 preserves legacy tag 7 semantics; that step raised the store/service floor to V6.
> Import Raft compaction, the full-image ceiling, sustained resource budgets and deployment gates
> remain open.

> **2026-10-05 follow-up:** [Import Raft compaction](RAW-TTL-IMPORT-COMPACTION.md) now discovers
> unbound TTL children with verified import plans and selected checkpoints. Actual three-replica
> planning, confirmation, physical reclamation and authorized recovery complete before Finish;
> a lagging learner blocks truncation. The image ceiling, sustained budgets, merge and deployment
> qualification remain open.

> [Per-SST checkpoint spooling](RAW-TTL-SPOOLED-CHECKPOINT.md) subsequently removes whole-image
> serialized buffers from both production checkpoint workers and journal recovery/upload. Actual
> three-replica import checkpoints complete with a buffer budget below one image's size. The 48 MiB
> format ceiling, resident indexes, sustained resource budgets and full production gates remain open.

> [V2 large checkpoints](RAW-TTL-LARGE-CHECKPOINT.md) now extend actual metadata/data workers
> beyond 48 MiB using tag 14 and a V7 store/service floor, preserving legacy tag 7/13 decoding.
> Three-replica import reclamation/reopen and subsequent bound-image recovery pass. Shared
> anonymous spool admission is present; durable disk debt, resident indexes, large-image
> capture/migration, merge and full production qualification remain open.

> [Persistent pending-file admission](RAW-TTL-PENDING-DISK.md) now accounts for named journals,
> publication temporary files and open unlinked readers across runtime groups, including startup
> inventory and authorized retirement. Exact durable plans can settle even above a lowered limit.
> This does not yet bound WAL/Raft, the full filesystem, CPU/RSS or sustained workload debt.

> [Atomic data admission](RAW-WRITE-ADMISSION.md) now counts unstable and uncommitted entries
> under the Raft append lock, preserves typed batch refusal and gates TTL split import copies.
> Runtime data groups default to 65,536 retained entries; control/recovery work can still advance.
> This is a per-group admission threshold, not a node WAL/Raft byte quota or production capacity
> qualification. Automatic compaction/reclamation still requires its existing configuration.

> [Node WAL/Raft byte admission](RAW-WAL-RAFT-BUDGET.md) now shares actual file debt and
> pending-position reservations across metadata/data groups, including recovery, temporary
> rewrites and authorized retirement. Client/owner drop cannot free accepted work. The default
> threshold is 4 GiB; follower/control persistence can exceed it. Filesystem headroom, remote
> pressure coordination and sustained capacity qualification remain open.

> [Local disk headroom admission](RAW-DISK-HEADROOM.md) now samples the owned Linux
> filesystem before data append, preserves 256 MiB by default and deducts shared pending
> claims plus overlapping writes. Probe failures/stale results refuse data; low-space
> followers still apply accepted entries. This is not preallocation, remote pressure
> coordination or a qualified guarantee of control/recovery space.

> [Remote replica pressure feedback](RAW-REPLICA-PRESSURE.md) now combines authenticated
> exact-store reports, bounded shared-maintenance polling and data-group append checks over
> the actual full configuration. Errors and stale replies cannot clear known pressure.
> Metadata keeps local admission so catalog repair can proceed. Reports reserve no remote
> bytes; full control/recovery space and sustained multi-group capacity remain open.

> [Streamed Raft rewrite workspace](RAW-RAFT-REWRITE-WORKSPACE.md) removes full-tail
> encoding copies, reserves exact temporary output before mutation and shares future
> space with data admission/remote reports. A separate 512 MiB default workspace permits
> reclamation above the data-debt threshold; unavailable space defers the group. Resident
> index size, full control/recovery space and remote shared reservations remain open.

> **Raw split retry fix (2026-10-04):** [Dedicated population commands](RAW-SPLIT-POPULATION-FENCE.md)
> refuse delayed copies after child ownership binds, preserving overwrites and deletes across
> three-replica WAL reopen. Legacy log semantics are unchanged. TTL transfer, complete import
> recovery, resource budgets and mixed-binary upgrade qualification remain open.

> **TTL phase in progress (2026-10-04):** [time/format, snapshot reads, replicated apply
> and bounded cleanup candidates](RAW-TTL-PHASE2.md) have component tests, including
> three in-process voters across a leader change. Lean lemmas cover the time and
> single-key cleanup core only. [V4 store/wire fencing and individual format confirmations](RAW-TTL-UPGRADE.md)
> and [configuration-bound activation](RAW-TTL-ACTIVATION.md) have implementation and local
> component coverage. Cross-node activation/topology acceptance, complete public routing, clock-provider
> validation, background cleanup and group storage recovery remain open, as do earlier prerequisites.
> [Linux TTL clock and replicated observations](RAW-TTL-CLOCK.md) now have local component
> coverage and explicit configuration; deployment clock qualification remains open.
> [Scoped public TTL RPC/stream, client/CLI and bounded scan](RAW-TTL-PUBLIC.md) now have
> local Raft/WAL and HTTP/2 duplex coverage. [Public activation administration and remote
> metadata directory lookup](RAW-TTL-ACTIVATION-API.md) now use the all-replica gate, with
> three-voter HTTP/2 coverage and separate metadata/data leaders. [TTL discovery and cross-range
> scan cursors](RAW-TTL-ROUTING.md) now have bounded client/CLI paths and local component coverage.
> [Full catalog bounds and paged DeleteRange](RAW-TTL-DELETE-RANGE.md) are now supported by
> public protocol v2, with exact per-page receipts and WAL reopen/three-replica coverage.
> [Bounded background TTL observation and conditional cleanup](RAW-TTL-MAINTENANCE.md) now have
> shared per-node TTL slots, round-robin scheduling, cancellation drain, unresolved-append fencing,
> WAL reopen and three-replica component coverage. [Data-group checkpoint recovery](RAW-TTL-CHECKPOINT.md)
> now validates the restored TTL image, exact publication and WAL tail; three real replicas reopen
> after engine segment reclamation. [Checkpoint/compaction synchronization and historical configuration
> preservation](RAW-TTL-CHECKPOINT-HISTORY.md) now protect recovery evidence across local prefix
> deletion and physical Raft log rewrite, with three-replica dual-WAL recovery coverage.
> [Cross-node checkpoint owner registration](RAW-TTL-CHECKPOINT-OWNERS.md) now has a bounded
> internal RPC and metadata-leader routing, with eight applied-but-unconfirmed handoff prefixes
> and actual data/metadata disk reopen tested with distinct leaders. [Automatic data checkpoints](RAW-TTL-DATA-CHECKPOINT.md)
> now publish/adopt with bounded jobs, durable candidate history reservation, interrupted-plan recovery,
> cancellation drain and three-replica segment reclamation coverage. Complete owner lifecycles, portable recovery anchors,
> whole-node maintenance budgets, steady resource acceptance,
> TTL split/migration, full NodeRuntime and network acceptance remain open;
> the complete phase and production gates have not passed.

> [Shared maintenance admission](RAW-TTL-SHARED-MAINTENANCE.md) now bounds seven maintenance
> classes with four node jobs, two heavy-I/O jobs, dispatch rate and bounded class waiting.
> Two blocked uploads leave TTL progress, and cancellation retains permits until actual I/O exits.
> Remaining maintenance paths, byte/memory/CPU budgets, physical debt and full-runtime acceptance stay open.
> [Object payload admission and bounded checkpoint GETs](RAW-TTL-OBJECT-IO.md) now add a shared
> runtime byte budget, including conditional-PUT comparison reads and cancelled admission waits.
> Local WAL bytes, temporary memory/CPU, physical debt and real MinIO acceptance remain open.

> [Checkpoint planning and journal memory](RAW-TTL-CHECKPOINT-MEMORY.md) now check full SST
> size before copying entries, stream the compatible pending format, and drop the original
> plan before reload. Shared temporary-memory admission and sustained RSS acceptance remain open.

> [Shared checkpoint image admission](RAW-TTL-IMAGE-BUDGET.md) now reserves exact serialized
> plan/pending bytes across runtime groups and retains reservations through actual upload return.
> Codec scratch, network copies, restored/resident indexes and whole-node RSS remain outside this budget.

> [Object buffer admission](RAW-TTL-OBJECT-BUFFERS.md) now follows shared PUT/GET byte owners,
> including references retained by queued jobs, SDK payloads and read results after caller return.
> Checkpoint reads preserve these owners; legacy Vec copies, codec scratch and restored indexes remain outside the cap.

> [Borrowed SST validation and direct restore](RAW-TTL-SST-VIEWS.md) now remove bulk parser
> key/value copies and the full restore WriteBatch, retaining the remote owner through iteration.
> A later SST failure drops all unpublished state; writer scratch, resident indexes and RSS acceptance remain open.

> [Bounded manifest encoding](RAW-TTL-MANIFEST-ENCODING.md) now enforces the 1 MiB frame
> limit before output growth, without a separate full JSON Vec or a format/change-ID change.
> Input/decode allocations, history counts and whole-node memory admission remain open.

> [Borrowed SST encoding](RAW-TTL-BORROWED-WRITER.md) now writes frozen rows directly into
> final buffers after exact image admission, removing the production planner's owned-entry collection.
> V1 bytes, partitioning and hashes are preserved; resident indexes, snapshot retention and full RSS remain open.

> [TTL image installation and learner recovery](RAW-TTL-INSTALL.md) now accept validated
> clock/expiry/history records and reject activation removal or watermark rollback on replacement.
> A live voter/learner capture-install-tail-reopen component test passes; transfer authority,
> full migration/topology qualification, split/merge and real network/MinIO acceptance remain open.

> [Adopted-generation recovery](RAW-TTL-ADOPTED-RECOVERY.md) now validates committed WAL
> positions, each range/TTL transition, later checkpoint publication and pending history protection.
> Restart uses the actual compacted base under committed catalog authority. The original remote image
> remains a recovery dependency; successor anchors, owner release and full transfer acceptance remain open.

> [Durable adoption TTL facts](RAW-TTL-ADOPTION-FACTS.md) now remove that old-SST read dependency
> for a later validated checkpoint. The V2 marker records the original image's verified TTL state;
> V1 upgrades revalidate the original closure, and old V1 readers refuse V2. Real local restart tests
> delete superseded objects before recovery. No retention owner is released; production GC, portable
> successor authority and old-binary/rolling-upgrade acceptance remain open.

> [Checkpoint settlement budgets](RAW-TTL-SETTLEMENT-BUDGET.md) now prioritize exact-window
> history and limit fallback effect lookup to four rows per resumable page. Each slot permits one
> active driver; old calls cannot clear or unlock replacement slots. A real three-voter test settles
> an old losing pending while retaining its global pin. Executor drain, negative owner release,
> GC, total history size and node-wide CPU/memory acceptance remain open.

> [Checkpoint local object draining](RAW-TTL-OBJECT-EXECUTION.md) now keeps maintenance permits
> and per-group execution state until actual queued object jobs exit, even after a synchronous
> caller times out. Real data/metadata dispatchers and the object worker are tested together with
> InMemory SDK transport. Local draining does not certify remote PUT cancellation or global
> executor closure; durable negative-Pending release and GC remain open.

> [Durable checkpoint executor owners](RAW-TTL-EXECUTOR-OWNERS.md) now register each actual
> uploader's node/store/process lifetime before I/O. After local drain, the exact Build generation
> transfers protection to its still-Published Pending/Version; an unconfirmed close blocks new
> uploads by that worker. Lost receipts, generation fencing and metadata WAL reopen are tested.
> Complete publisher registration/closure, crashed-owner retirement, history capacity and negative
> Pending release/GC remain open; no global object protection is removed by this change.

> [Per-owner retention rows](RAW-TTL-OWNER-ROWS.md) remove the production ledger's 4096-owner
> lifetime ceiling per resource, preserving Released generation fences and an immutable bounded
> base for legacy records. WAL tests retain 4097 released owners at constant per-operation point
> work; remote metadata upgrade survives a lost receipt and WAL reopen. Legacy-base decoding,
> physical history growth, safe history reclamation and full upgrade qualification remain open.

> [Durable checkpoint intents](RAW-TTL-CHECKPOINT-INTENT.md) now retain canonical descriptors
> and predecessor generations in metadata Raft before object I/O. A bounded authenticated lookup
> survives local journal clearing and metadata WAL reopen, including losing attempts. Seven Lean
> component lemmas and refusal controls pass; TLA+ parses but TLC/refinement remain open. This
> supplies recovery information for publisher closure, not negative-release or deletion authority.

> [Checkpoint admission closure](RAW-TTL-CHECKPOINT-ADMISSION.md) now durably rejects entry and
> Publish after closure, indexes bounded executor pages, and fences unentered generations.
> Lost receipts and metadata WAL reopen preserve closure; automatic data recovery resolves a
> closed losing journal from committed history without object I/O. Eight Lean component lemmas
> and refusal controls pass; complete legacy backfill, old-binary qualification, crashed executors,
> global closure certificates, negative Pending release and GC remain open.

> [Unpublished executor retirement](RAW-TTL-UNPUBLISHED-RETIREMENT.md) can now retire an exact
> indexed Held generation behind a closed gate, atomically retaining independent evidence.
> Recovery examines four executors per pass and survives lost receipts and data/metadata WAL
> reopen; Published/Quiesced executors and global object coverage remain. Nine Lean component
> lemmas cover delayed receipts and distinguish ordinary Release from retirement evidence.
> Published crash leftovers, complete closure and physical reclamation stay open.

> [Global checkpoint discovery and durable sweep progress](RAW-TTL-CHECKPOINT-SWEEP.md) now
> find gated attempts without local journals. The metadata leader rotates one attempt/four owners
> per pass through an eighth shared maintenance class. Lost retire/cursor receipts, leader transfer,
> metadata WAL reopen and damaged-value isolation have real Raft/duplex coverage. Done only ends
> the indexed Held sweep; Published executors and global protection remain. Legacy attempts without
> gates, complete publisher termination, negative release and physical history capacity stay open.

> Exact committed-window reconciliation now completes positive Pending-to-Version handoff and
> closes negative admission. Missing local history can use an authenticated, bounded remote query;
> the caller rechecks endpoint generation before owner mutations. Real Raft/duplex tests cover a
> metadata leader without a target-group lookup handle and a remote follower holding the history.
> Unknown replies retain protection; complete placement/deployment qualification and negative
> Pending, old Version and Reader release remain open.

> [Per-generation executor drain observations](RAW-TTL-EXECUTOR-DRAIN.md) now bind actual
> object scopes to exact ledger tokens and retain immutable issued/resolved PUT counts after
> local drain. Later successful generations cannot erase old unknown writes; an unconfirmed
> report blocks the same worker even after Release. Real queue, Raft/duplex, three-generation
> and WAL-reopen checks pass. Missing crash reports, unknown remote PUT fencing, backend
> qualification, complete publisher closure and negative Pending release remain open.

> **Local retirement update (2026-09-27):** [worker/inbox reuse and payload draining](GROUP-RETIREMENT.md)
> have component coverage. The [2026-10-03 task-history update](TASK-HISTORY-PAGINATION.md)
> removes the 255-row lifetime ceiling while preserving all receipts; local churn
> now passes with the complete catalog. Indexed maintenance budgets, history GC
> and full fault acceptance remain open; production capacity is not yet accepted.

> **Prior development direction (2026-09-16): horizontal scaling.** The user ended the
> current performance iteration at the validated CRC/rpds baseline. Redis parity
> and radix completion are not prerequisites. Start dynamic multi-Raft (#22),
> routing (#23), replica movement (#24), recoverable split (#25) and measured
> placement/scale-out (#27). See the [implementation and benchmark contract](HORIZONTAL-SCALING-PLAN.md).
> Acceptance requires actual 3/6/9-node throughput/latency scaling, strict
> consistency proofs, actual Chaos Mesh and no database service singleton.
> Earlier experiment reports below are retained historical evidence.

> The [radix cursor/range proof](RADIX-CURSOR-PROOF.md) adds 80 theorems,
> with all 287 model theorems checked together and 25 new rejecting controls.
> Complete forward/reverse bound filtering and arbitrary mixed-direction range
> histories now refine independent entry-list specifications, including no
> duplicate keys, exhaustive/fused behavior and inclusive predecessor maximality.
> Actual Rust loop/vector/frame simulation, Arc/COW heap correspondence and
> source differential execution remain open before timing. The unchanged
> prototype retains its 213-test qualification (25 existing ignored), six Rust
> fault controls and 54 allocation observations. No radix speedup, new database
> QPS or runtime promotion is claimed; production keeps rpds. See #51.
> Latest direction: [optimize writes against three-copy Redis](WRITE-PERFORMANCE-NEXT.md).
> Keep selected ThinLTO and Safe ReadIndex; read and lease optimization remain held.
> The latest [index mutation screen](OUTLINED-MUTATION-PATH.md) completes 446
> dependency tests, 26 conditional lemmas, four engine smokes and 528 timing/count
> rows. Original overwrite mean falls 18.220% / 11.893%, but unique insert gains
> are smaller and pinned p99 regresses. Warm small-map GET hit regresses in both
> orders; the declared component gate fails. The [bounded layout diagnosis](READ-LAYOUT-DIAGNOSIS.md)
> reproduces the gap; fixed function alignment does not fix it. Four independently
> checked first-map snapshots show matching structure and relative layouts within
> each configuration. The [completed engine-interface screen](ENGINE-INTERFACE-SCREEN.md)
> passes 90 processes / 152 rows and independent checks: write mean improves
> 4.600%–13.903%, but warm owned GET hit regresses 2.599%–7.407%; resident reads
> do not share the same uniform regression. Keep the candidate isolated and the
> original failed gate. The [triomphe-only performance screen](ISOLATED-OUTLINE-PERFORMANCE.md)
> now completes 136 processes / 152 timing / 76 allocation rows. All allocator
> windows match. Write means improve 10.803% / 3.690% for overwrite and
> 4.250% / 1.526% for unique insertion; small resident warm hit regresses.
> Its material/read gates fail too. Stop these callback variants. The
> [inline-key write screen](INLINE-KEY-PERFORMANCE.md) now completes 52 processes /
> 32 timing / 16 count rows. Short means improve 1.685%–10.831%, but three miss
> the material gate; long unique means regress 5.077% / 4.783%. Stop before the
> predeclared read stage and hold inline40. The new buffer qualification above
> checks layout, key-only ordering and replacement/snapshot semantics. Its
> matched write screen now fails as recorded above.
> Owned-buffer removal stays held.
> Database QPS below is unchanged; no industrial checkbox or runtime promotion follows.
> CRC remains the selected write runtime. The latest [same-source screen](WAL-PREALLOCATION-PERFORMANCE.md)
> measures default `86aa6fc` at c64 / 128-byte values at **137,874 Put calls/s**
> and **1,022,750 BatchPut(64) items/s**, with p99 **0.745–0.754 ms** and
> **9.306–9.437 ms** respectively. WAL preallocation improves **0.545% / 1.474%**
> pooled, with unchanged loaded pooled p99 and order-dependent batch direction.
> Keep the feature default-off; do not repeat the unchanged screen. All eight
> smokes, sixteen timed cohorts and independent checks pass: **7,200,959 timed
> calls / 63,451,454 items**, 48 drains, 64 exited timed lifetimes and complete
> retained-byte verification. This measures current checkpoint-owner integration;
> earlier comparisons retain their own source and client attribution. The
> [Raw apply experiment](RAW-APPLY-ATTRIBUTION.md) now joins exact original commands
> and engine groups. Direct append lowers conversion mean 10.896%, saving only
> 1.548 us/group, with lower p99 in both orders. The subsequent
> [composed MemEngine screen](RAW-LOWERING-COMPOSED.md) holds this prototype:
> overwrite mean -1.134% / -0.452% without/with snapshots, initial-fill unpinned
> p99 +3.309%. All state/snapshot checks pass; the performance gate fails. The
> [first packed-node implementation/screen](PACKED-INDEX-EXPERIMENT.md) also
> fails selection: overwrite +26.957% / +12.403%, despite new-key gains. Semantic,
> snapshot, allocation and independent dataset/statistic checks pass. Diagnose
> comparator/minimum-refresh work and the 24-byte shared prefix before a changed
> kernel; preserve the fixed-stride read-sampling limitation and use unbiased
> future query selection. Production and database QPS are unchanged.
> The [completed observer capture](WRITE-OBSERVER-CAPTURE.md) identifies repeated
> receipt misses. The independent [upper-bound candidate](WRITE-RECEIPT-UPPER-BOUND.md)
> now passes 14 source-bound theorem statements / 82 obligations, local
> development checks and [retained releases/ordinary recovery](WRITE-RECEIPT-UPPER-BOUND-RUNTIME.md).
> Both complete recovery histories pass: 360 operations, including 30 explicit
> unknowns, six fresh drains and seven exited lifetimes. [Actual full21 Chaos](WRITE-RECEIPT-UPPER-BOUND-CHAOS.md)
> also passes: 9,639 complete operations, four fresh final drains, 36 exited server
> lifetimes and 25 exited containers. The [actual schema-2 observer](WRITE-RECEIPT-UPPER-BOUND-OBSERVER.md)
> also passes all 16 short cohorts: at c64 Put, about 88% of lookups skip the scan.
> New bulk outputs use `/mnt/data/kv9-work`, with separate root/data/tmpfs guards.
> Capacity is sufficient. The missing historical benchmark client has an explicitly
> [requalified replacement](WRITE-CLIENT-REQUALIFICATION.md): exact source/toolchain
> checks, 60 source/report tests and all eight actual smokes with independent
> dataset/accounting/lifetime checks pass. All sixteen timed cohorts and the
> enclosing audit now pass: 7,154,151 one-attempt successes / 62,959,110 items,
> 64 exited timed lifetimes, 48 fresh drains and all original retained bytes.
> C04 now has [automatic checkpoint ownership](CHECKPOINT-OWNERS.md): durable exact
> upload bytes, replicated Pending before remote I/O and Version handoff after
> positive settlement. The 731-test local library gate, strict composition proofs
> and one actual default Chaos Mesh leader-failure cell pass. The model also covers
> refusal after prior application, with an explicit ordered reachability witness.
> Next run the separate pre-upload crash cut, then integrate typed negative
> history and certified abort release, legacy coverage/fencing, reader drainage
> and destination installation. C04 and physical object deletion remain open.
> The [retained batch-tail analysis](write-batch-tail-review-v1/README.md) now
> narrows the missing observation to group preparation/apply/receipt insertion
> and terminal inspection. [Default-off bounded timestamps](WRITE-STAGE-TRACE.md)
> implement those exact-position joins, with the diagnostic scope retained below.
> The first [matching release pair and actual capture](WRITE-STAGE-CAPTURE.md)
> exposed exporter/recording contention: five lost calls correctly refused the
> instrumented row. A tested correction snapshots between existing Raft pumps.
> The [corrected four-row capture](WRITE-STAGE-CAPTURE-RESULTS.md) now passes:
> 131,737 calls / 8,431,168 items, zero recording loss, 12 drains and 16 exited
> lifetimes. Combined observer features cost 2.770% pooled throughput in this
> short diagnostic. Deduplicated leader group apply means are 549.018 / 546.521 us;
> locks and receipt publication/inspection are much smaller in the retained tails.
> The [resident-index experiments](RESIDENT-INDEX-EXPERIMENTS.md) now reject
> sorted coalescing and two-pass value reuse. A one-traversal borrowed upsert
> passes 53 dependency tests, but remains held: jemalloc overwrite index time
> improves 12.269% while pure insertion regresses 12.121%. Source patches and the
> exact corpus are retained; no runtime integration or new database QPS is claimed.
> Its static follow-up found no supported correction; stop that variant.
> The [WAL payload preallocation candidate](WAL-PAYLOAD-PREALLOCATION.md) now
> reuses validated record size, behind a default-off feature. Eleven capacity
> theorems/seven controls, the existing 47 CRC theorems/eight controls, and
> both 851-test workspace/Clippy configurations pass locally. Large-batch
> encoder mean time improves 27.339%; all six small cases also improve mean/p99.
> These are CPU microbenchmarks, not database QPS. [Matching releases and ordinary
> recovery](WAL-PREALLOCATION-RUNTIME.md) now pass: 728 complete operations
> (666 OK / 62 unknown), twelve fresh drains and fourteen exited lifetimes.
> [Actual candidate Chaos Mesh](WAL-PREALLOCATION-CHAOS.md) now passes all 21
> windows, 9,241 complete operations, four final drains and 31 exited server
> lifetimes. The [completed end-to-end screen](WAL-PREALLOCATION-PERFORMANCE.md)
> retains small point gains and inconsistent batch gains; the feature stays off.
> Retain prior rejected buffer/index experiments and the pending
> pre-upload crash acceptance, then continue industrial storage,
> resource-bounded multi-Raft, ownership changes and automatic splits with the
> original dependencies. Proof, actual Chaos Mesh and no critical singleton
> requirements remain mandatory. CI stays local.
Updated: 2026-09-16. This file defines delivery order. `DESIGN.md` preserves the long-term architecture;
[TAKEOVER-AUDIT.md](TAKEOVER-AUDIT.md) maps that architecture to the current implementation.

The execution breakdown is in [DEVELOPMENT-PATH.md](DEVELOPMENT-PATH.md), with 25 work packages,
explicit dependencies, implementation steps and acceptance criteria. Track delivery in
[GitHub issue #9](https://github.com/c4pt0r/kv9/issues/9).

The qualification history below retains its original revisions and decisions;
the latest selected write runtime and next action are stated above.

The [earlier checkpoint](CURRENT-STATUS.md) records accepted ThinLTO full72
performance and the exact-build Chaos matrix, including 9,923 complete history
operations. The 36 smoke / 72 timed point/batch cohorts and independent audit
now pass. Main now selects the exact qualified source after fresh local release,
observer-feature and default-build/recovery checks. The retained server/client
bytes match the original candidate. Preserve the loaded batch-write p99
regression. The [completed fixed-rate diagnosis](BATCH-WRITE-FIXED-RATE-RESULTS.md)
retains higher whole-call and scheduled p99 at both offered rates and both
orders, with 2,044 client-dropped slots preventing strict matched-work inference.
All 797,956 issued calls succeed; no performance cohort was rerun. Client timer
calibration is a separate environment question. The [quorum-path capture](QUORUM-TRACE-RESULTS.md)
now preserves the original lossy recorder result and a qualified immutable-slot
repair. Six repaired prefixes have zero loss, 2,318 matched leader-local round-trip
candidates and 1,159 exact group chains. The 17.286 / 17.102-us round-trip means
include transport and remote work; 0.285 / 0.297-us confirmation-to-eligibility
means retain the original successful pump/apply fence. This is diagnosis, not
a new selected runtime speedup. The [32-us owner-poll source checkpoint](https://github.com/c4pt0r/kv9/blob/2ca5fccb157b26b6c3c79eb52f7c7838f10a5c8c/docs/BOUNDED-OWNER-POLL.md)
now passes 14 new TLAPS theorems / 49 obligations, model/negative controls,
714 default tests and 443 standalone read-stage tests, with overlapping
populations. Original failed drafts and command/build-inventory failures remain
retained. The [clean default release and ordinary recovery](OWNER-POLL-RECOVERY.md)
now pass, including 363 complete operations, 29 unknown outcomes and seven
exited lifetimes. The [24-cohort c1/c64 GET/mixed screen](OWNER-POLL-PERFORMANCE.md)
now passes accounting but rejects the candidate: every workload/order loses
throughput and worsens mean/p99, with more server CPU. Main still selects
`11113f6`. Reuse existing profiles and capture a concrete selected-build CPU cost
before another rewrite; do not retune this poll budget or repeat losing cohorts.
Keep notification candidate `42e0117` separate; its gains cannot be added to
ThinLTO's measurements. The [next latency investigation](QUORUM-LATENCY-NEXT.md)
targets unresolved intervals inside a fresh quorum round, reusing completed
body-handoff evidence and preserving all earlier rejected experiment decisions.
Preserve fresh quorum reads, sealed groups, successful pump/apply/view fences
and durable acknowledgements.
The separately requested lease path now has a complete conditional mathematical
argument, eight checked TLAPS lemmas / 23 obligations and real-clock SMT
containment. The [fixed-configuration transition proof](LEASE-AUTHORITY-MODEL.md)
adds 27 theorems / 348 obligations and bounded fault-model evidence for renewal,
revocation, restart promises and the local view gate. The
[Rust component](LEASE-CONTROLLER.md) passes 205 Raft library tests, source fault
controls and integer-timing proofs. Its [voting adapter](LEASE-VOTE-BINDING.md)
now binds durable peer installation and actual elections. The
[renewal adapter](LEASE-RENEWAL-PUBLICATION.md) adds exact envelopes, leader lifetime
binding and whole-pump certificate publication, with 231 passing Raft tests.
The [read-view integration](LEASE-READ-VIEW.md) now binds fresh commit capture,
the exact owned applied view, final authority and the original admission budget
to experimental GET/BatchGet. Its local gate passes 586 tests (one existing
server test ignored) and nine compiled source fault controls. Unary/streaming
RPC traversal and deferred same-view metadata/value checks pass. Default startup
remains Safe ReadIndex. Expired authority plus unavailable quorum fails closed.
The [Linux clock adapter](LEASE-CLOCK.md) now checks an explicit drift/error
contract against the durable policy. Its three arithmetic proof queries and
three countermodels, 593 library tests and actual process-pause expiration pass.
The held lease path still requires physical clock/host/VM qualification and
actual fault histories before any lease performance selection. See [the proof and implementation gates](LEADER-LEASE-PROOF.md).
The current product sequence is write performance against Redis with one primary
and two replicas, then dynamic multi-Raft and automatic splits, retaining all
storage/proof/fault prerequisites. Read parity is no longer the active gate.
Earlier rejected executor/handoff experiments retain their decisions. Evaluate
throughput and latency together, including mixed-read tails. CI runs locally
except at releases or explicitly selected key milestones.

The target is an industrial-grade distributed database. Prioritize consistency, recovery, measured throughput
and scalable architecture. Complex private-network TLS configuration is not a prerequisite for the current
milestones. Existing identity, root descriptor, token authentication and epoch checks remain part of the baseline.

Hard gates for every stage: rigorous core-protocol proofs with explicit implementation refinement,
real Chaos Mesh fault-injection E2E, and no service-critical single point of failure except the object-store
dependency. See [CORRECTNESS-GATES.md](CORRECTNESS-GATES.md) for proof obligations, evidence requirements
and the distinction between replica failure and host-failure isolation.

## Current baseline: working distributed Raw KV

- One `META_REGION_0` Raft group replicates both self-hosted metadata and Raw KV. Real multi-process tests cover
  bootstrap, failover, restart, learner registration and promotion.
- Writes pass through Raft; data and exact `(term,index)` share a WAL v2 record and fsync before publication.
  Raw and public catalog reads establish a quorum ReadIndex barrier.
- The catalog has typed tables, indexes, foreign keys and transaction overlays. Production mutations acquire
  the catalog mutex, commit an ordering barrier, plan against a stable snapshot and propose in the same leader term.
- With `KV9_STORAGE=minio`, a leader freezes full state, uploads immutable SSTs, confirms PUT and readability,
  and proposes manifest CAS. Each replica adopts only after local ordered apply, persists a recovery pointer,
  and reclaims the covered catalog WAL prefix.
- Cold recovery reads MinIO SSTs and replays the surviving WAL tail. Pending flushes verify cluster and committed
  history, recheck remote bytes, and recover their original identity before another attempt.
- Legacy in-band applied markers can be validated against committed Raft history and upgraded atomically.
  When the legacy full state exceeds the 64 MiB record limit, the old prefix remains and only a verified position
  transition is appended, preserving the ability to reopen large old stores.

This is a basic implementation, not an industrial capacity or performance claim. The entire dataset remains in RAM,
and resident planning/capture has a 48 MiB ceiling; production workers now use
[bounded V2 streamed checkpoints](RAW-TTL-LARGE-CHECKPOINT.md) for larger images.
Normal catalog WAL reclamation now publishes a segmented
topology and removes covered closed files; the legacy fallback still copies the surviving tail before rename.
Raft prefixes can be reclaimed through guarded compaction; complete historical manifest/SST GC remains open.
Multiple running data groups have implementations and component evidence; the Raw/TTL split
prototype was moved to the `experimental/range-split` branch on 2026-10-28.
Incremental LSM, block cache, TTL merge and full production qualification remain future work.

Acknowledged unflushed writes depend on the replicas' local durable logs. A bucket alone cannot reconstruct cluster
identity, protocol state or an unuploaded tail.

## P0: make consistency and recovery continuously verifiable

Complete this gate before capacity or throughput claims:

1. Add named cuts around WAL append/fsync, Ready persistence/send, upload confirmation, manifest apply,
   checkpoint pointers and WAL replacement. Extend deterministic and real-process fault matrices to disk-full,
   EIO, reordered/duplicated messages, partitions/healing and interrupted membership changes.
2. Record concurrent Raw KV and catalog histories and run an independent linearizability checker.
   Distinguish success, proven refusal and unknown outcomes. Matching final values is insufficient.
3. Preserve the delivered `catalog.pending` recovery and exact historical-winner reconciliation.
   Missing files or a latest pair beyond the reconciliation window cannot prove failure.
   Integrate retained evidence with snapshot/GC pins before deleting history.
4. Define backup/recovery anchors and format compatibility. WAL v2 currently requires an offline upgrade;
   mixed-version rolling upgrade and downgrade are not promised.
5. Retain selected counts, failure cuts, logs and exclusive completion markers. Keep real MinIO process tests
   in the PR checks and establish metrics and repeatable single-group benchmarks early.

**Exit gate:** under a documented failure model, acknowledged writes survive, deletes do not reappear, minority
leaders cannot establish successful reads, and replaced log positions cannot produce false success receipts.
Bad objects or protocol anchors fail recovery explicitly. Each covered cut has reproducible positive and negative
controls; uncovered power-loss/fsync behavior remains identified.

The bounded public admission increment (#38) is followed by fixed local latency
observations (#39), documented in [LATENCY-OBSERVABILITY.md](LATENCY-OBSERVABILITY.md).
The client foundation for C03 (#40), including its retry/deadline proof and real
connection tests, is documented in [PERSISTENT-CLIENT.md](PERSISTENT-CLIENT.md).
The remaining workload increment must record end-to-end
latency independently, and separate refused requests, unresolved writes and
acknowledged operations before any throughput claim. Instrumentation itself is
not benchmark acceptance; exact-revision evidence is tracked on the issues.

## P1: bounded storage and measured throughput

1. Implement active/immutable memtables, incremental SSTs, stable versioned views, range tombstones, leveled
   compaction and a bounded block cache. Read remote SST blocks directly so memory no longer scales with live data.
2. Complete qualification of the implemented segmented WAL and whole closed-segment reclamation.
   Add atomically installable Raft snapshots before protocol-log truncation.
   Follow the [dual-WAL decision](ADR-DUAL-WAL.md); any unification must preserve atomic data/position recovery
   and the [shared recovery/retention contract](RECOVERY-RETENTION-CONTRACT.md).
3. Qualify the implemented proposal batching and group commit, and bound upload concurrency and queues.
   Propagate admission/backpressure for slow disks, slow MinIO and compaction debt.
4. Benchmark fixed CPU/RAM/disk/network/MinIO topologies, key/value sizes, distributions and concurrency.
   Report throughput, p50/p95/p99, error rates, fsync latency, apply lag, backlog, amplification, memory and recovery.
   Measure one group before many groups; do not invent QPS targets.
5. Enable actual SST deletion only after references, reader/checkpoint/snapshot pins, orphan handling and durable
   delete intents are implemented. Pending proposals, transfers and backups must retain their inputs.

**Exit gate:** data larger than memory remains serviceable; memory and disk usage stay bounded across multiple
compaction cycles; recovery converges within measured limits; performance improvements pass the same P0 correctness
suite and publish reproducible before/after results.

## P2: multiple Raft groups and scale-out

1. Separate metadata and user data groups with RegionManager, shared transport, batched scheduling,
   per-group lifecycle and explicit resource budgets.
2. Bind catalog regions to real groups. Implement routing-cache invalidation, epoch fences, leader hints and
   bounded client rerouting. Define cross-region Raw operation semantics before exposing them.
3. Attach learners through snapshots and shared SSTs, catch up tails, and change membership safely after log
   truncation. Recover ConfState, data and position consistently.
4. Implement durable pre-shard, split, merge and migration protocols before automatic placement.
   Detect throughput/queue hotspots and use hysteresis and budgets to avoid oscillation.
5. Measure lazy attachment and cold-cache costs. Claim metadata-only transfer only when byte measurements
   exclude a hidden full copy. Introduce L0/L1 metadata sharding only after measuring the bottleneck.

**Exit gate:** independent hotspots benefit from added nodes; migration, split and membership changes preserve
single ownership and complete coverage. Shared SST attachment transfers metadata and an unflushed tail, with
object-store reads reported separately.

## P3: tenant service quality

Transactions (MVCC, TSO, Percolator SI, lock recovery, deadlock detection) were removed from the design on
2026-10-28. The multi-key write primitive is the atomic range batch (`RawBatch`, DESIGN §9.1), which is already
implemented on the raw path; what remains here is service quality, not a new consistency model.

Add tenant admission, weighted fairness, cache quotas, background-work attribution, resource accounting and
bounded tenant metrics. This work can overlap late P2 once the shared scheduling interfaces are stable.

**Exit gate:** hot tenants and slow uploads do not indefinitely starve others; SST GC, snapshots and long-running
batch workloads share an accountable retention protocol; a range batch that crosses a boundary is refused with the
typed signal under split and membership churn (chaos-verified).

## P4: operational completeness

Deliver complete backup/PITR, versioned migrations, verified rolling upgrades, operational APIs, diagnostics,
capacity guidance and long-duration soak. Run short fault/nightly tests from P0; P4 adds sustained release acceptance.

Add deployment-specific TLS/mTLS, certificate rotation, cloud identities and finer authorization here.
Keep an explicit HTTP MinIO/token-authenticated development workflow so core protocol testing stays accessible.

Isolate MinIO behind `ObjectStore` and the maintained S3 client. Pin the test image digest and verify at least one
maintained S3-compatible service or cloud S3 before production backend claims.
The [official MinIO repository](https://github.com/minio/minio) was marked archived when checked on 2026-09-08.

## Existing issues and execution planning

The takeover checked [#5](https://github.com/c4pt0r/kv9/issues/5),
[#6](https://github.com/c4pt0r/kv9/issues/6), [#7](https://github.com/c4pt0r/kv9/issues/7) and
[#8](https://github.com/c4pt0r/kv9/issues/8). Their implementation evidence is in
[TAKEOVER-AUDIT.md](TAKEOVER-AUDIT.md). Publishing code, observing CI and closing an issue are distinct steps.

Bounded storage, recoverable log compaction, measurement and multi-group ownership precede tenant service quality.
Complex private-network TLS configuration remains later work.
