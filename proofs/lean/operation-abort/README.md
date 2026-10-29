# Operation-abort model

Finite-trace projection of one split or merge intent and its abort (docs/RANGE-SPLIT.md §12):
abort rows, seal evidence rows, publication, membership moves on the parent and the retirement
of abandoned children. The catalog's rule is the definition `effective`: an abort row and no seal
evidence.

| Model element | Implementation |
| --- | --- |
| `abort` | `plan_range_abort` (kind 117): refuses once seal evidence exists; a repeat confirms |
| `seal` | `seal_parent`: re-checks the abort, proposes the CAS, commits seal evidence (kind 118) |
| `publish` | `plan_split_publication` / `plan_merge_publication`: refuse an effective abort |
| `moveParent` | `plan_migration` through `live_range_operation`: refused while an unaborted, unpublished intent names the group |
| `retireChildren` | `reconcile_retirement` on `aborted_children` |

Ten theorems: safety initialization/preservation/reachability; one abort per operation; a published
operation's abort is ineffective; a move needs the abort; children retire only on abort; an abort is
refused after a seal; a late seal makes the abort ineffective; an abort frees the parent. Five
semantic controls and two policy controls.

Source-mapped abstract reasoning with explicit premises: not verified Rust extraction; drains and
replica plans (kind 117 before capture, kind 108 after) are covered by their own planners' tests.
