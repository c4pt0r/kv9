# Single-file WAL segment protocol

This proof covers the `WalSegment` primitive introduced in
`738a029abcd521b1e4a08acb5fb50bb443e5b47e`. It does not prove stream topology,
rotation, checkpoint adoption, reclamation, legacy migration or whole-engine
recovery. Those remain integration obligations under #15 and #14.

## State and assumptions

The [TLA+ model](../proofs/tla/segment/SegmentDurability.tla) describes one file
already created and synchronized, selected by external durable topology and
protected by exclusive store ownership. A file-identity token represents the
complete expected stream/sequence/predecessor header. Frame tokens represent
batch payloads together with optional exact applied positions; separate data
and position maps let the proof reject a writer that splits that binding.

`written` counts complete frames visible to the current process. `durable`
counts the prefix covered by successful synchronization; `summary` is the
published local summary, and `ack` retains the greatest caller acknowledgement.
Counts never include an incomplete tail. Arbitrary unacknowledged complete frames
may survive a failed synchronization or crash. Recovery may publish them after
validation and synchronization; it does not pretend they were acknowledged.

The theorem parameters permit arbitrary positive capacity, frame identities,
position assignments and selected-file identities. The two finite TLC instances
are counterexample searches, not the deductive proof. Index gaps and arbitrary
interleavings of positioned/unpositioned records are allowed. Indexes must grow
and terms must not regress across positioned records. Unpositioned records do
not erase the preceding position and remain represented in the summary's pin.
The mathematical frame count models record boundaries; byte-offset arithmetic,
integer-overflow refusal and the Rust decoder remain source-level obligations
covered by the implementation and its filesystem regressions.

Explicit premises:

- One owner controls the file; external topology, caller locking and creation
  durability establish that ownership before this protocol starts.
- The complete expected header is checked before any record becomes recovery
  state. Checked framing/decoding detects the modeled malformed, mismatched or
  mixed record; arbitrary checksum collisions are outside the model.
- A successful file/namespace synchronization preserves its covered prefix
  through the modeled crashes. Failure has an unknown persistence result.
- Recovery visitors build unpublished state. Any later validation or I/O error
  discards that state and returns no usable writer. The proof does not make an
  arbitrary caller's externally visible callback transactional.
- Replicated apply supplies truthful data/position pairs. This component checks
  their local ordering and atomic persistence; it does not prove Raft commitment
  or authorize checkpoint installation.

The trailing frame CRC binds the segment header, frame header and payload.
Each nested header CRC field is excluded from this trailing computation: including
a message together with its CRC can cancel its contribution through a fixed CRC
residue. The independent frame-header checksum still validates length before
allocation. Actual swap/transplant regressions cover these boundaries; no theorem
claims that CRC-32 is collision-free or authenticates hostile storage.

## Safety and conditional progress

The [parameterized TLAPS proof](../proofs/tlaps/segment/SegmentDurabilityProof.tla)
establishes:

1. Acknowledgements and published summaries never exceed the synchronized frame
   prefix. Every acknowledged frame's exact data and position remain unchanged
   across subsequent appends, failures, crashes and recovery.
2. Each complete frame retains its data/position binding. Positioned records
   obey strict index and nondecreasing-term order; the exact summary preserves
   all unpositioned pins.
3. A usable writer requires the selected identity, complete successful validation
   and completed synchronization. Failed writes or synchronization fence the
   handle; only dropping/crashing and opening a new recovery attempt leaves it.
4. Active recovery discards only an incomplete, unacknowledged final fragment.
   Complete corruption or a wrong header rejects the entire attempt. Validation
   failures leave the file unchanged. A later repair/synchronization failure may
   leave a repaired tail, but publishes no writer or recovered state. A later
   crash may restore the old incomplete tail until repair synchronization succeeds.
5. Sealing consumes writer authority. Closed replay preserves the file's complete
   frame content and does not grant an active writer. Closed-length/summary or
   record validation failure is a rejected observation.

`SGLastGuardEquivalence` proves that the model's all-prior-position guard equals
the runtime's last-position comparison on an ordered prefix. Its transitivity
lemma requires a positioned middle record; an unpositioned record cannot supply
a false transitive watermark. The header predecessor supplies the last position
when the segment has no positioned record. `SGSummaryFoldCorrect` proves by
natural-number induction that every finite prefix folded through the runtime's
first/last/pin update rules has the exact first and last positioned records and
retains every unpositioned pin. Its initial and step lemmas also cover empty
prefixes and runs of unpositioned records between positioned records.

The induction uses `NatInduction` from the pinned standard `NaturalsInduction`
library. The semantic inventory records that library and its `Integers`
dependency alongside `Naturals` and `TLAPS`; these standard mathematical
libraries are part of the stated proof-tool trust boundary.

Progress is conditional on an operation already being in flight or a recovery
scan already being opened. Fair local execution and successful I/O, with no new
crashes/failures, eventually settle it as a ready writer, closed result or explicit
rejection. There is no timeout or availability claim under endless faults, missing
storage, a caller that never opens recovery, or absent upstream quorum service.

## Source mapping

| Model boundary | Implementation at `738a029` |
| --- | --- |
| `SGInit`, selected identity | `WalSegment::create` synchronizes header and namespace; `scan` compares the exact expected `SegmentHeader::encode` bytes before calling the visitor. Creation/topology selection are explicit entry premises. |
| `SGStart` | `append` checks `summary.last.or(header.previous)` and payload/count limits before file mutation. `SGLastGuardEquivalence` relates its last-position guard to the model's ordered-prefix guard. |
| `SGSummaryFoldInitial`, `SGSummaryFoldStep`, `SGSummaryFoldCorrect` | `SegmentSummary::add` preserves the first positioned record, advances the last positioned record, and accumulates the unpositioned pin. The fold proof justifies the summary used by append and scan. |
| `SGWrite` | Header, batch and checksum form one checked frame; recovery accepts the complete bound frame or returns no usable writer. Partial physical writes are represented by the separate tail/failure transition. |
| `SGSync`, `SGPublish` | `append` calls file synchronization before assigning `self.summary` and returning success. A crash after sync but before the return may recover an unacknowledged complete record. |
| `SGFail`, `SGCrash` | Write/sync errors set `poisoned`; subsequent append/seal refuse. Dropping the handle and `recover_active` create the next attempt. Unknown unsynchronized suffixes may survive or disappear. |
| `SGOpen`, `SGValidate`, `SGReject` | `scan` validates header, bounded framing, checksums, decoding and position order before its successful return. Wrong identity, complete corruption or visitor failure does not truncate or return a writer. |
| `SGRepair`, `SGRecoverySync`, `SGRecoveryPublish` | `recover_active` truncates only the validated incomplete tail, seeks the append boundary, synchronizes file/namespace, then returns the writer and exact summary. |
| `SGSealStart`, `SGSealSync`, `SGClosedReplay` | `seal(self)` consumes the writer and synchronizes before returning `ClosedSegment`; `ClosedSegment::replay` checks published length, complete scan and exact summary without modifying the file. |

The model's `bad` observation includes complete malformed records, closed-summary
mismatch and failed recovery visitation. It does not assume that a corrupted file
still contains usable acknowledged data; it proves that such an observation grants
no recovery authority. The frame maps describe the correctly bound logical prefix
against which a successful read is validated.

## Verification gate

```sh
python3 scripts/check-segment-protocol.py \
  --tlapm /path/to/pinned/tlapm \
  --jar /path/to/tla2tools-v1.7.4.jar \
  --output /tmp/segment-protocol-new
```

The gate checks exact source/tool inventories, every named declaration and exact
obligation count. Every proof uses strict mode and a fresh cache. It rejects
omitted proofs, custom axioms, incomplete module inventories, truncated successful
output and missing/incomplete temporal checks through the shared audited validators.

Fourteen isolated protocol controls cover split data/position frames, overwritten
first-position summaries, last positions erased by unpositioned frames, early
acknowledgement, reusable failed writers, loss of an acknowledged prefix, bypassed
position validation, wrong-file acceptance, complete-corruption acceptance,
premature repair, unsynchronized recovery publication, lost unpositioned pins,
reopened closed writers and missing recovery fairness. Each control checks original,
one-source mutation and restored source through TLC and TLAPS. Counterexamples
must reach the intended property; compilation, syntax, timeout and unrelated
errors do not count as successful negative controls. By default, four independent
control groups run concurrently, each retaining its own baseline, single-source
mutation and restoration sequence, copied sources, fresh proof cache and logs.

Two capacities use different predecessor contexts and two independent TLC
fingerprints. A separate fair-continuation model starts at concrete operation and
recovery cuts. Three reachability witnesses require acknowledged records,
recovered unacknowledged records and retained unpositioned pins to be reachable.
The gate supplements, and does not replace, the primitive's actual filesystem
regressions or the later integration's required Chaos Mesh acceptance. Routine
verification remains local; no hosted workflow is dispatched by this script.

## Retained local acceptance (2026-09-09)

The complete gate in `/tmp/kv9-segment-protocol-third` exited successfully with:

| Check | Accepted evidence |
| --- | --- |
| Parameterized proof | 45 declarations and 469 obligations; 33 independent positive runs with strict mode and fresh caches |
| Finite exploration | Capacity 2: 2,264 states; capacity 3 with a predecessor: 2,187 states; each agrees across two fingerprints |
| Conditional progress | 21 states from nine concrete operation/recovery cuts; removing recovery-sync fairness produces a temporal counterexample and the intended failed proof |
| Protocol controls | All 14 baseline/mutant/restored triples pass in both TLC and TLAPS |
| Reachability | Acknowledged records, recovered unacknowledged records and unpositioned pins are each reachable |
| Gate controls | Both semantic faults and all eight incomplete-output controls are rejected |
| Independent replay | All 50 TLC cases, 49 proof/audit cases, copied-source hashes, tool hashes, named failed obligations and restoration results agree |

The retained archive is
`target/correctness-evidence/2026-09-09-738a029-segment-proof.tar.gz`:
1,925,398 bytes, 1,102 entries, SHA-256
`498a293afd09368c4ff660eb1e928af837ac87305f57a8d65a518efb35c44e78`.
Every archived file was read back and compared with its recorded source bytes.
Its manifest binds the exact proof/model/script sources, the unchanged primitive
at `738a029`, all final logs and the independent replay reports.

Earlier development failures remain in the archive. The first complete-gate
attempt rejected a stale inventory of 384 obligations after proving 388; the
second was deliberately interrupted to add the missing scalar-summary induction.
Neither is counted as final acceptance. The final gate has the corrected exact
inventory of 469 obligations and includes the two additional summary controls.
This proof-only acceptance did not rerun Cargo, MinIO, Chaos Mesh or hosted CI.
