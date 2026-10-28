# Can bbtree replace the red-black tree under `MemEngine`?

Date: 2026-10-28. Evaluated: [bbtree](https://github.com/XiangpengHao/bbtree) 0.1.6 (Xiangpeng Hao;
[post](https://bbtree.xiangpeng.systems/post/), MIT/Apache-2.0, `no_std`, zero runtime dependencies) against
the `rpds::RedBlackTreeMapSync` that backs every column family of `crates/engine/src/mem.rs`.

**Short answer: not as a drop-in replacement. bbtree is a faster ordered map, but it is a different kind of
map.** `MemEngine` is built on a *persistent* (structurally shared) tree: `Engine::snapshot` is an O(1) clone
that pins a version, every read takes such a snapshot, and writes copy only the paths they touch. bbtree is a
mutable, in-place, latch-based B-tree: its `clone` is a deep copy, its scans are explicitly "not a snapshot",
and it has no versioned view at all. Swapping the type would silently break the read model (§2). Where bbtree
does fit is as the *mutable layer* of the incremental-LSM memtable that README already lists as future work
(§4).

## 1. What bbtree is

- An in-memory B-tree with 16–32 entries per node, linear/SIMD search over **inline keys** (the first 2–8
  bytes of each key after the node's shared prefix; `Vec<u8>`, `&[u8]`, `String`, integers are specialised,
  `InlineKey` for user types), batch lookups that overlap cache misses with a tiny hand-rolled async
  executor, and a **version lock** (optimistic lock coupling) on inner nodes so that concurrent readers write
  no shared cache line. Freed nodes stay on a per-map free list, so a late optimistic reader never touches
  unmapped memory.
- Two maps: `bbtree::unsync::BBTreeMap` (single-threaded, API mirrors `std::collections::BTreeMap`) and
  `bbtree::BBTreeMap` (shared by threads; `insert`/`remove`/`get_cloned`/`read(&k, |k, v| …)`/`scan`/
  `range_cloned`). `bbtree-store` layers byte-string keys and values in budgeted heap pages on top;
  `bbtree-mmap` maps a built tree from a file.
- Correctness story: Verus proofs for the inline-key ordering, fuzzing against `BTreeMap`, Shuttle schedule
  exploration, TSan/Miri CI. The code is LLM-written and the author is the reviewer of record; the
  [HN thread](https://news.ycombinator.com/item?id=50223777) reports mixed results on other people's
  workloads (a 3-D mesh kernel got slower; `String`-keyed maps used ~40% more memory than `BTreeMap`).

## 2. What `MemEngine` needs that bbtree does not provide

| Need in kv9 | How `MemEngine` gets it today (rpds) | bbtree |
| --- | --- | --- |
| **O(1) snapshot that stays fixed under later writes.** Every `Engine::get/scan` clones `State`; ReadIndex reads, routed scans, the checkpoint freeze, TTL sweeps and migration capture hold a view for seconds to minutes while apply continues. | Persistent tree: clone = three `Arc` bumps; writers copy touched paths. 200k writes under a held snapshot cost 287 ns each and retained 53 MiB. | `Clone` is a node-by-node deep copy (132 ms and +212 MiB per 1M entries, measured); `scan`/`range_cloned` are documented as "not a snapshot: an entry inserted or removed while the scan runs may or may not be visited". No versioned read exists. |
| **Atomic multi-CF visibility.** One `RwLock<State>` over the three CFs makes a catalog batch (row + index rows) and a Raw `WriteBatch` visible all-or-nothing. | A single write lock around the batch; readers hold an immutable `State`. | Each map is linearizable per operation; a batch is a sequence of individually visible inserts unless an external lock serialises readers too, which gives back the contention bbtree exists to avoid. |
| **Owned view, never a lock guard** (the driver lock-order invariant pinned by `_assert_read_returns_owned_snapshot`). | The owned `State` clone. | `read(&k, f)` runs the closure under the leaf latch; `get_cloned` copies one value. An owned *view* of the whole map does not exist. |
| **Deterministic replay to an exact `(term, index)`** and cheap "is this view at position X" checks. | `applied` travels inside `State`. | Would need an external version; see §4. |

So the question "replace the red-black tree with bbtree" is really "replace a persistent map with a mutable
map", and the engine's read model is what pays.

## 3. Measurements

Micro-benchmark on this host (32 cores, release, `-C lto=thin`; source `/mnt/data/kv9-work/bbtree-bench`):
1M entries, kv9-shaped keys (`r\0\0d` + `user/` + 12 digits, 30 bytes with a 16-byte shared prefix),
128-byte values, random insert order, 1M random probes (half misses), 20k scans of 100 rows.

| map (1 thread) | insert ns | get ns | scan-100 ns | remove ns | live MiB | writes under a held snapshot |
| --- | --- | --- | --- | --- | --- | --- |
| `std::BTreeMap` | 508 | 549 | 1852 | 522 | 200 | 166 ns, after a 123 ms full clone |
| `rpds::RedBlackTreeMapSync` (kv9 today) | 1138 | 1060 | 5455 | 1033 | 232 | 287 ns, clone 0 ms (299 ns with no snapshot held) |
| `bbtree::unsync::BBTreeMap` | 559 | 224 | 878 | 417 | 212 | 125 ns, after a 132 ms full clone |
| `bbtree::BBTreeMap` (shared, 1 thread) | 344 | 264 | 790 | 452 | 212 | no snapshot API |

Concurrent point reads over the same 1M entries, as kv9 performs them today (RwLock read + O(1) clone per
read) versus bbtree's lock-free `get_cloned`:

| threads | rpds behind `RwLock`, clone per read | bbtree `get_cloned` |
| --- | --- | --- |
| 1 | 1428 ns/op (0.7 M/s) | 274 ns/op (3.7 M/s) |
| 4 | 406 ns/op (2.5 M/s) | 84 ns/op (11.9 M/s) |
| 8 | 195 ns/op (5.1 M/s) | 63 ns/op (15.8 M/s) |

Reading: bbtree is 4–5× faster on point reads, ~6× on short scans and 2–3× on inserts in a single thread, and
~3× on 8-thread reads. Memory is *not* lower for byte-string keys and values (payload is ~158 MiB of the
total; overhead per entry: std 42 B, bbtree 54 B, rpds 74 B). The one row where rpds wins is the only one kv9
relies on: a snapshot costs nothing and a writer under a snapshot pays 287 ns instead of 299 — the price of
persistence is paid at every write (~2× std) rather than at snapshot time.

Caveat: this is the map alone. In the running system the engine map is behind a WAL write, a Raft commit and
a quorum ReadIndex; kv9's measured write path is fsync-bound (`docs/benchmark` history), so the single-thread
insert gain would not show end to end, while the read-side gain could, because a ReadIndex barrier serves
many reads from one view.

## 4. Where bbtree could fit

1. **As the memtable of an incremental LSM engine** (the direction README's limits paragraph already names).
   Writes go to one *mutable* bbtree; `snapshot()` freezes it (make it read-only, start a new empty bbtree)
   and a view is `{frozen layers…, current}` merged on read. Freezing is O(1), frozen layers are immutable
   so readers need no latch, and the per-write persistence tax disappears. The multi-CF atomicity moves to
   the freeze boundary (a batch is applied entirely to one mutable layer under the existing lock). This is
   a new engine, not a type swap: `ReadView` becomes a layered merge, `delete_range`/scan merge tombstones,
   and the checkpoint writer flushes frozen layers to SST. It is also exactly the shape that lets SSTs become
   the read path later.
2. **Not** as a replacement inside today's `MemEngine`: without snapshots every reader would have to hold the
   write lock for the duration of a scan or checkpoint, which is a regression in both latency and the
   lock-order invariant the driver depends on.
3. **Not now for memory**: for kv9's byte keys and values it uses more than `std` and only 9% less than rpds.

## 5. Risks if adopted

- Version 0.1.x, edition 2024, API still moving (`AGENTS.md` asks users to file regressions); depends on a
  date-pinned `vstd` for proofs (ghost code; compiled fine on stable 1.94 here).
- Optimistic readers + in-place mutation is a class of bugs kv9's persistent map cannot have; bbtree's own
  tests found one race only after Shuttle missed it (issue #9). Our fence/ReadIndex evidence would need
  re-establishing on the layered engine.
- Integer-key results in the author's charts do not transfer to byte keys (HN; our table): measure on kv9's
  keys before believing a ratio.

## 6. Recommendation

Keep `rpds` under `MemEngine` on `master`. Open the incremental-memtable engine as its own design packet with
bbtree as the candidate mutable layer, gated on the same acceptance that governs the engine today (fence
tests, ReadIndex partition evidence, checkpoint/recovery e2e). Until that exists, the measured headroom is a
reason to do that work, not to swap the type.
