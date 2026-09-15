# Resident-index allocation experiments

Updated 2026-09-15. **No runtime candidate is selected.** Sorting/coalescing and
two-pass value reuse are rejected. A one-traversal borrowed upsert saves time on
overwrites, but its pure-insert regression still prevents integration. The
production engine, dependencies, WAL and Raft behavior remain unchanged. The
only production-file edit corrects inaccurate snapshot-cost documentation.

Follow-up: static inspection of the retained jemalloc executable shows that
the borrowed insertion function contains allocation/copy work performed by the
old caller. Its larger symbol has different responsibilities, so symbol size
does not identify the cause of the insertion regression. No supported corrective
change was found; stop advancing this variant and preserve all completed runs.
The next [WAL payload allocation experiment](WAL-PAYLOAD-PREALLOCATION.md) changes
a different, explicit allocation site. No new resident-index profile or timing
was collected for this follow-up.

## Evidence and input

The corrected write-stage capture locates a larger apply interval, but does not
attribute its complete CPU cost. Offline inspection of the existing 3,106-sample
profile finds insertion/comparison work, with incomplete callers: none of the
271 insertion leaves or 123 comparison leaves below insertion identifies a
named outer caller. Only one of 128 iterator stacks identifies a routing caller.
The 87 `Arc::make_mut` leaves do not establish actual cloning. These historical
samples are not new CPU percentages for the current executable.

Source inspection confirms that validation snapshots are released before write
completion; no unnecessary request-lifetime snapshot was found. Actual retained
snapshots can force shared tree paths to be copied. Snapshot creation is O(1),
but the old claim that snapshots cost nothing to writers was incorrect.

We selected node 2's segment 10 from the first default row of the
[corrected capture](WRITE-STAGE-CAPTURE-RESULTS.md) before decoding it. Its
16,520,140 bytes match the original retention hash
`87aef6bfacdb58dd5e271db5ab96f3d16a03a4264c1594b7bf2503b6660b37fa`.
All frame checksums and increasing positions pass offline decoding. The segment
contains **106 batches / 100,096 mutations**, with 64–2,688 mutations per batch.
**14,448 updates (14.434%)** are overwritten within their own atomic batch.
This is one interior segment, not a complete timed client history or a new
recovery/commit-authority check.

The [packet](resident-index-experiments-v1/README.md) includes the exact derived
batch corpus, prototype sources and original measurements. The decoder preserves
every frame in the chosen segment; it does not select batches based on results.

## Experiments

Each comparison runs original/candidate/candidate/original in one release test
executable on CPU 4 of the shared host. Each row performs 12 passes over 106
batches: **1,272 batch timings / 1,201,152 mutations**. Inputs are prepared before
timing. A separate case holds one owned snapshot across each batch update. There
is no network, RPC, WAL, Raft or client queue in this experiment. Snapshot drop,
initialization and corpus decoding are outside the timed interval.

1. **Sort and coalesce:** sort at most 8,192 mutation indices by physical key
   and descending ordinal, then keep the last update per key. CF is part of the
   identity, scratch is bounded, and small/oversized batches use original order.
   Exhaustive short histories and boundary tests pass. With the system allocator,
   pooled index time increases **25.263%** without a retained snapshot and
   **48.943%** with one; p99 worsens in both orders. Sorting costs exceed the
   work saved. Reject this implementation before runtime integration.
2. **Two-pass reuse:** `get_mut` and `Vec::clone_from` reuse an existing value;
   misses call ordinary insertion. Snapshot/value tests pass. System-allocator
   retained overwrite time falls **19.627%**, but pure-insert time increases
   **38.921% / 31.080%** without/with a snapshot. A miss walks the tree twice.
   Reject this implementation.
3. **One-traversal prototype:** a borrowed upsert follows the original insertion
   traversal and balancing. An initial `make_mut` version also copies a shared
   old value before overwriting it. Its complete system-allocator measurements
   remain in the packet. The final correction uses `get_mut` for exclusive entry
   access, otherwise allocates the replacement entry directly. Its final paired
   experiment uses **jemalloc 0.6.1**, matching the server's allocator selection.
   Different allocator experiments are not pooled or treated as a controlled
   measurement of the correction alone.

Final jemalloc results below show pooled mean-time changes. Lower is better.
Per-order p99 values remain separate in the raw results; they are not averaged.

| Workload | Snapshot held across batch | Mean index-time change |
| --- | --- | ---: |
| Retained overwrite corpus, prepopulated | No | **−12.269%** |
| Retained overwrite corpus, prepopulated | Yes | −0.956% |
| Retained corpus, initially empty each pass | No | −5.124% |
| Retained corpus, initially empty each pass | Yes | +0.948% |
| Unique inserts throughout each pass | No | **+12.121%** |
| Unique inserts throughout each pass | Yes | +1.736% |

For prepopulated overwrites without snapshots, means are
**116.396 → 94.976 µs** in the first pair and **99.833 → 94.725 µs** in the
reverse pair; p99 changes **333.264 → 263.112 µs** and
**278.902 → 250.228 µs**. The changing baseline also shows why the pooled number
is a short shared-host observation, not a stable production speedup.

For pure inserts without snapshots, means worsen
**319.522 → 347.277 µs** and **305.290 → 353.268 µs**; p99 worsens
**962.802 → 977.289 µs** and **902.268 → 1,061.256 µs**. The unique-insert
case rewrites each mutation's final eight key bytes with a unique identifier,
keeping key/value lengths and original batch boundaries. The initially empty
retained corpus alone would miss this regression because most later operations
overwrite its 4,096 keys.

## Correctness and limits

The final candidate passes **53 local dependency tests**: 52 existing red-black
tree tests and one new differential test. The new test uses the actual ArcTK
map, compares complete tree structure, colors and size against ordinary insertion,
checks all existing red-black invariants, mixes removals and varying value lengths,
and retains old snapshots for comparison. The engine-level helper test also
compares against `BTreeMap`, covering empty keys/values and retained views.

This is an algorithm prototype. The active engine never calls the new API, and
the patch is applied only to an isolated copy of pinned, MIT-licensed rpds 1.2.1.
Preparation from its registry archive reproduces both modified source files
byte-for-byte. Original failed test-environment attempts are retained: Cargo
first discovered an archival parent manifest, then offline resolution found a
missing locked dependency. An isolated workspace declaration and an exact locked
dependency fetch resolved those issues; the suite passed once afterward.

There is **no discharged new mechanized proof, Raft integration, ordinary
recovery, Chaos Mesh acceptance or end-to-end performance result** for this
candidate. No full workload matrix was rerun. Current database QPS and Redis
comparisons retain their previous provenance and values.

## Refinement argument for subsequent verification

For byte-vector keys and values, erase allocation identity from a tree and retain
its keys, values, colors and child structure. Structural induction compares the
borrowed upsert with the existing `insert_mut(key.clone(), value.clone())`:

- An empty child creates the same red/black leaf and returns the same new-key flag.
- At a smaller/greater key, both algorithms descend into the same child. The
  induction hypothesis gives the same updated child and flag. Both call the
  unchanged balancing function exactly when the flag is true and blacken the
  root identically.
- At an equal key, byte-vector ordering equality means identical key bytes.
  `clone_from` leaves the new value's bytes in an exclusively owned entry; the
  shared-entry branch creates the same replacement as ordinary insertion.
  Children and colors are unchanged, and both return false.

Thus the erased tree and size update agree, conditional on the existing map's
balancing and shared-pointer contracts. For old snapshots, every modified path
is first made exclusive through the existing copy-on-write API; the entry is
mutated only when `get_mut` grants exclusive access. Otherwise the shared entry
is left intact and replaced in the live path. No `strong_count`-based guess or
unsafe access is added. This is a written refinement argument, not a machine-
checked proof of the dependency or its allocator/ownership implementation.

Any eventual engine integration must preserve original batch order, complete
WAL bytes, validation-before-apply, atomic cross-CF state/position publication,
revision accounting and durable Raft acknowledgements. It still needs the
source-bound proof and actual recovery/Chaos gates, plus representative point,
batch, insert, overwrite, snapshot and mixed-read performance checks.

## Next action

Keep the selected runtime. Do not repeat sorting/coalescing or two-pass reuse.
The borrowed upsert is held on its reproducible pure-insert regression. Inspect
its new-key allocation/code-generation cost before proposing a changed kernel;
if that cost cannot be removed, reject it rather than advancing it through an
expensive end-to-end gate. Do not extrapolate these index timings into database
QPS or weaken consistency to obtain a gain.
