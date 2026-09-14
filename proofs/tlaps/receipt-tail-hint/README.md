# Checked receipt tail-hint refinement

The candidate adds a checked constant-time hint before the earlier vector
binary-search experiment. It starts from selected CRC runtime `bd42e60`, without
combining the unmeasured FNV writer. The current profile attributes 5.094% of
point samples to the original linear receipt search; this is a hypothesis for
optimization, not a prediction of database throughput or latency.

For strictly increasing receipts, subtract the query index from the tail index.
If that distance fits `usize` and is less than the retained length, inspect
`len - 1 - distance`. Return the complete stored receipt only when its index
equals the query. A gap can make this hint wrong; failed validation falls back
to the original candidate's sorted slice search. A duplicate or decreasing
append permanently clears the ordering certificate and uses the baseline's
oldest-first linear search. No producer ordering assumption is introduced.

The original Vec append/trim operations, capacity 1,024 and exact tuples remain.
The source contract reverses only the container adapter, constructor, module
import and lookup expression, then requires byte equality with the entire
selected parent's driver. Fatal checks, lock ordering, exact term/outcome
interpretation, eviction uncertainty, watermarks, notifications, admission,
quorum, synchronization and response eligibility remain in that unchanged text.

## Universal proof and source mapping

`AppliedReceipts.tla` and its proof are byte-identical to the accepted earlier
receipt model. For every finite input trace and positive capacity they prove
retained sequence equality, boundedness and the checked ordering certificate.
The historically deque-named sequence is only an auxiliary logical sequence:
`ARPushSequenceEquality` equates it with the actual append-then-trim vector.
The candidate still uses a Vec.

The new model uses one-based hint position `Len(s) - (last.index - key)`.
Validation requires that position to be in the retained range and its stored
index to match. Strict ordering gives uniqueness, so any validated hint returns
the original first matching full tuple. Invalid hints use the inherited sorted
search contract. A false certificate uses the original first-match expression.
The refinement therefore preserves both receipts and absence for arbitrary
traces, including duplicates, gaps, eviction, terms and exclusive verdicts.

Rust's checked subtraction rejects queries above the tail, which produce a
model position above `Len(s)`. A failed `usize` conversion cannot produce a
valid slot: any valid distance is below Vec length and thus representable.
The bounds guard makes both usize subtractions safe. The array index equality
check supplies `RTHValidatedMember`; strict uniqueness establishes whole-receipt
identity. A final theorem proves the hint selects the exact position for a
contiguous suffix. Density is only a cost premise, never a safety premise.

The inherited 13 statements have 122 obligations; five new statements have 36.
Both modules are checked freshly with strict/no-fingerprint TLAPS, including
the imported proof module. SANY audits all transitive imports, the theorem
inventory, approved parameter assumptions and absence of proof holes.
The gate also runs finite capacity-one/two models, two incorrect algorithm
controls, restored source, proof-hole/false-axiom controls and six output
rejection controls. Finite exploration supplements the parameterized proof.

The trusted boundary includes Rust primitive/container operations, successful
allocation, slice binary search's sorted-input contract, existing caller
synchronization and the proof backends. This is not whole-Rust or whole-Raft
verification, nor a crash, liveness or database performance result.

Run from this source checkout with a fresh output path:

```sh
PYTHONOPTIMIZE=0 PYTHONDONTWRITEBYTECODE=1 taskset -c 6-15,22-31 \
  python3 scripts/check-receipt-tail-hint-proof.py \
  --tlapm /path/to/tlapm --jar /path/to/tla2tools-v1.7.4.jar \
  --output /path/to/new-receipt-proof-evidence
```

The source contract pins all implementation/model/proof/helper inputs. Original
commands, sources, counterexamples, semantic audits and terminal results remain
in the output. Existing output directories are refused. Rust source checks,
exact-release process recovery, actual Chaos Mesh and matched throughput/p99
qualification remain separate gates before any promotion.
