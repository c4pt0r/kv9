# Parameterized metadata proofs

These proofs import the same `MetadataPlanning.tla` used by TLC. They do not
translate its transitions into a second state machine. The inventory contains
41 theorem declarations and 459 checked proof obligations across nine modules.

| Module | Theorems | Obligations | Result |
|---|---:|---:|---|
| `Collections` | 5 | 19 | Sequence/function range closure, including append, prefix restriction and point updates |
| `MetadataShape` | 3 | 35 | Initialization, transition preservation and temporal induction for log shape and committed/applied bounds |
| `MetadataPrefix` | 2 | 29 | Every transition preserves committed entries; the canonical TLA+ action property holds throughout every specified execution |
| `MetadataReceipt` | 9 | 109 | Request field preservation, written-entry correlation and `Spec => []ReceiptSafety` |
| `MetadataPlanningControl` | 2 | 28 | Per-host planner exclusion, planning-term bounds and current-term ownership |
| `MetadataPlanningBarrier` | 2 | 62 | No later write before current-term submission; exact observed barriers remain in applied prefixes |
| `MetadataFreshness` | 8 | 69 | Equality of the local and retained write cuts; `Spec => []FreshPlans` |
| `MetadataAllocation` | 3 | 35 | Checked natural induction, bounded maximum existence and the actual `LastWrite` choice |
| `MetadataUniqueness` | 7 | 73 | Positive ordered write IDs, unique names and `Spec => [](LogSafety /\ CatalogSafety)` |

`ReceiptAlways`, `FreshAlways` and `CatalogAlways` apply to arbitrary legal
coordinator/request sets and term budgets, and to all executions of the model,
including stuttering. These are deductive results, not enumeration of the two
TLC configurations. Their premises
include the model's abstract Raft contract. It does not prove that Rust, the
network, the filesystem or object storage implements that contract.

The full `TypeOK` bounds and conditional draining proof are **not** mechanized
by this inventory. `ShapeAlways` proves the specific log/index bounds
defined in `MetadataShape`, not every clause of `TypeOK`.

## Toolchain and reproducibility

The TLAPS Linux archive is pinned to the official `1.6.0-pre` build at upstream
commit `4600b24c6d95a25ff081ad37b63b2a01c29d43a5` (version output `4600b24`). Its
SHA-256 is
`bfa5e5350ac1ec7202feecad0a4a71a5bb58c16a49660448b35b6f371ba9e2f5`.
The rolling release URL may change in the future; a changed archive must fail
the checksum gate until a reviewed toolchain update. The bundled backend versions
include Isabelle2025 and Z3 4.8.9. PTL uses the bundled LS4 translation/backend.

With that archive extracted and the pinned TLC v1.7.4 jar available, run:

```sh
python3 scripts/check-tlaps.py --tlapm /path/to/tlapm/bin/tlapm \
  --jar /path/to/tla2tools.jar --output /tmp/kv9-tlaps-evidence
```

The output directory must not exist. Each case copies the source inventory,
imports the actual model, parses it independently with SANY, and invokes TLAPS
with `--strict --nofp` and a new cache directory. Every module must report its
exact nonzero obligation count without error/warning diagnostics. The full run
checks every imported project proof module before publishing an accepted result;
an imported theorem declaration alone cannot discharge its proof obligation.

The semantic audit checks module dependencies, the exact theorem inventory,
proof holes, nested modules/instances and module-level assumptions. The model's
one existing input-assumption block is explicitly fingerprinted. Added axioms or
assumptions are rejected even if indented; they are not detected by a line-based
keyword convention. Standard module sources are also fingerprinted. SANY enforces
TLA+ syntax and semantic levels separately from TLAPS; in particular a temporal
action property must use the canonical `[][A]_vars` form.

Twelve isolated controls each require a passing baseline, a specific rejection and
a passing restored source: index-only acknowledgement, acknowledgement before
application, replacement of a committed entry, an omitted proof, a custom axiom,
an empty inventory, an invalid temporal action property, a missing planner mutex,
a ReadIndex-only wait, a missing submission term fence, a non-advancing allocator
and a root module that omits part of the inventory. Three output controls
reject exit-zero runs with empty output, zero obligations or a missing summary.
Parser/tool errors are not accepted as protocol counterexamples. Every mutation
owns exactly one file, and restoration is checked by source hashes.

Retained evidence includes the exact sources, assumption/module/theorem audit,
commands, logs, tool/source hashes, cache outputs and per-case results. A timeout
is inconclusive and fails the gate. A final `summary.json` exists only after all
proofs and controls pass.

## Trust and refinement boundary

The trusted base includes TLAPS parsing/elaboration and its backend translations,
Isabelle's logic/checker, Z3's SMT result and LS4's temporal result, plus the pinned
standard modules. This is not a claim that every SMT/PTL result has a Lean-kernel
or Isabelle proof certificate. Strict mode and negative controls do not eliminate
backend/compiler bugs. No project axiom, omitted proof or disabled checker is
allowed to close an obligation.

The main proof strengthens the receipt property with an inductive invariant:
well-formed logs and committed/applied bounds; valid request-control fields;
successful requests in a terminal phase; and each retained write's exact request,
index, term and planned value. That last relation matters because the receipt
guard checks identity, while the value was captured earlier at submission.
Only after all transitions preserve the strengthened invariant does temporal
induction establish the receipt theorem.

The planning proof adds the per-host mutex, current-term ownership, exact applied
barrier and absence of later writes before submission. These facts establish
snapshot equality and preserve current-term ready plans. A separate checked
induction proves that every nonempty bounded set of write indices has a maximum;
the model's `CHOOSE` therefore selects the last retained write. The allocator
remains the last blindly written ID plus one. Positive, strictly increasing IDs
and unique names form the additional inductive invariant for catalog safety.
Discarding an uncommitted suffix may reuse its IDs; acknowledged writes remain
protected by the prefix and receipt theorems.

Removing the term fence fails `Submit` preservation in `MetadataUniqueness`.
Freshness alone does not require a stale plan to become current again, so its
induction is not the load-bearing term-fence obligation. Similarly, removing the
mutex fails `Begin` preservation, and a ReadIndex-only wait fails exact applied
barrier preservation. These are specific failed proof goals, not a claim that
failure to prove an arbitrary statement is itself a counterexample; the TLC
controls retain concrete protocol traces separately.

The unchanged implementation mapping and open obligations are recorded in
[METADATA-PLANNING.md](../../docs/METADATA-PLANNING.md). Quorum/log matching,
Ready/persistence refinement, full catalog invariants, retry-loop refinement and
cross-host availability remain separate work. Actual Chaos Mesh history testing
continues alongside the proof jobs.
