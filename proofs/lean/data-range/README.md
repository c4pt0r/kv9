# Ordered data-range ownership and terminal sealing

`Range.lean` checks 16 statements about initial ownership, exact write scope,
whole-batch rejection and one-way sealing. Run with Lean 4.33.1:

```sh
python3 scripts/prove-data-range.py --lean /path/to/lean --output /fresh/output
```

The checker pins the sources, compiles with warnings as errors, audits every
theorem's axioms, rejects eleven semantic mutations and rejects two proof-policy
violations. Only standard Lean axioms are permitted. This is an abstract proof
with a reviewed implementation correspondence, **not verified Rust extraction**.

## Correspondence and premises

| Model | Implementation and premise |
| --- | --- |
| `Range.identity`, `space` | Exact root, creation digest and region in `DataRange`; keyspace/tenant are immutable. `RegionManager` configures the state machine from a validated durable creation before starting its driver. Metadata kind-102 tasks bind a new namespace to that creation. A decoded descriptor alone grants no runtime authority. |
| `install` | `Command::DataRange` tag 8 is interpreted only at ordered apply. An absent descriptor accepts initial unsealed epochs 1/1 with the exact configured identity. Identical-state confirmations preserve state; the model abstracts their receipts as stuttering. A different existing descriptor cannot be initialized again. |
| `closeRange` | Expected descriptor SHA-256 and `DataRange::may_follow` permit only the same identity, namespace and bounds with version incremented once and `sealed=true`. The model's equality assumes collision resistance. Rust rejects integer overflow; natural numbers model the nonoverflowing branch. There is no public seal/split/transfer API yet. |
| `allowed`, `write` | `RangeFence::is_fresh_write` rereads the owning engine at ordered apply, checks exact epochs and the unsealed descriptor, then checks every operation's Default CF, Raw prefix, keyspace and half-open logical bounds. Both ordinary and grouped apply use it. A rejected batch contributes no writes. Existing atomic positioned storage apply and Raft order are premises. |
| `contains` | Natural-number order abstracts lexicographic byte order. Empty upper bytes represent an unbounded upper limit; the lower endpoint is included and a finite upper endpoint excluded. Canonical decoding and physical-key prefix parsing are separately tested implementation obligations. |
| Read scope | `RawGroup` obtains a successful Safe ReadIndex barrier, captures one engine snapshot and authorizes/reads that same snapshot. The scope theorem relates authorization to the descriptor in that view. It does not prove the existing ReadIndex algorithm, snapshot implementation or end-to-end linearizability. A concurrent seal may follow a read's snapshot; a read whose barrier follows the committed seal must reject. |
| Proposal authorization | A private, non-cloneable `RangePermit` authorizes a proposed batch only. A seal between preparation and apply is caught by the ordered fence. DeleteRange retains one post-barrier selection view and acquires a fresh proposal permit for each chunk; receipts retain the existing partial-progress semantics. |
| `Step`, `Run`, restart | Once sealed, every modeled future installation, seal, write and restart preserves sealing. Durable range state and acknowledged data recovery rely on the existing WAL/Raft recovery contracts and exclusive store ownership. |

Malformed or foreign stored ownership is an apply error with no advanced
watermark, not a guessed stale/fresh verdict. Rust tests exercise that distinction
for ordinary apply, grouped apply and reopening. The model covers well-formed
state; filesystem, decoder and failure propagation correctness are premises.

Metadata's legacy Raw gate and ordered catalog fence reject keyspaces marked
`KV9DATA01`. Publication of a local dispatch handle occurs only after the data
group applies its own descriptor. A missing handle cannot authorize fallback
to metadata. V3 node-to-node methods and the durable `KV9LIFE3` writer floor
exclude earlier writers that do not implement these checks. Separate actual
binary upgrade tests support this requirement; it is not a cryptographic or
Byzantine proof.

## Limits

The current public mapping is one new full-keyspace range per data group.
Old keyspaces remain with their old owner. The trusted Raft command surface
still includes internal generic commands; public data routing submits only
fenced writes. The proof assumes compliant peers and the crash/partition model.
No extracted capability theorem forbids malicious internal command construction.

This increment does not prove or implement split publication, range movement,
replica replacement, stale-route retries, multi-endpoint discovery, cross-group
atomic transactions, lease reads, reclamation or horizontal throughput gains.
The single-view theorem is a scope property, not a new proof of Raft. Actual
Chaos Mesh, a history checker and independent-host scaling benchmarks remain
separate acceptance gates.
