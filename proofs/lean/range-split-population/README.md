# Range-split population and publication model

Finite-trace projection of one split after its intent (docs/RANGE-SPLIT.md §3–§5): whether both
children serve, whether the parent is sealed, how many rows of the sealed half one child has
copied, whether the copy is verified, how many evidence rows the child has, whether its sibling has one, and the one-transaction
publication with the routing that flips with it.

| Model element | Implementation |
| --- | --- |
| `serve` | both children with a leader and an applied half descriptor (`range_split.rs`, the child descriptor proposal) |
| `sealParent` | the parent leader's `DataRange` CAS to version+1 sealed, once both children serve |
| `parentWrite` | the apply-time fence: a sealed descriptor refuses every write |
| `copy` / `verify` | one `RawExecutor::scan` page from the local sealed parent replica into the child's own log; the row-for-row digest of parent half and child |
| `evidence` / `siblingEvidence` | `plan_split_evidence` (kind 114) once per child; a repeat confirms |
| `publish` | `plan_split_publication`: the parent binding sealed in place and both child bindings in one catalog transaction, only with both evidence rows; `ranges::read_in` refuses any partial shape |

Twelve theorems: safety initialization/preservation/reachability; evidence means a verified copy
of the whole sealed half; one evidence per child; the sealed parent accepts no write; a copy reads
only a sealed parent; the seal waits for the children; publication needs both children; routing
follows publication atomically; retirement needs publication; the chain reaches publication. Nine
semantic controls and two policy controls.

Source-mapped abstract reasoning with explicit premises: not verified Rust extraction; silent
about the digest function and about TTL-activated parents (whose seal the state machine refuses).
