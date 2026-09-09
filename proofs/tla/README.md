# TLA+ protocol models

TLA+ is the primary language for protocol state, transitions, safety and temporal
properties. TLC checks finite instances and produces counterexamples. Deductive
proofs in [TLAPS](../tlaps/README.md) now cover the metadata model's committed
prefix, exact receipts, planning freshness and name/ID uniqueness; remaining
protocol properties and the existing Lean lemmas retain their separate scopes.
Neither TLC nor an abstract theorem
mechanically verifies the Rust implementation.

## Reproduce the metadata checks

Use Java 21 and the official `tlaplus/tlaplus` v1.7.4 release's `tla2tools.jar`:

```sh
curl --fail --location --output /tmp/kv9-tla2tools.jar \
  https://github.com/tlaplus/tlaplus/releases/download/v1.7.4/tla2tools.jar
python3 scripts/check-tla.py --jar /tmp/kv9-tla2tools.jar \
  --output /tmp/kv9-tla-evidence
```

The output directory must not exist. The runner verifies SHA-256
`936a262061c914694dfd669a543be24573c45d5aa0ff20a8b96b23d01e050e88`
and the embedded TLC 2.19 version. This checksum pins our downloaded official
release artifact; it is not a separately published vendor signature.

`MetadataPlanning.tla` specifies the protocol. `MetadataMC.tla` supplies concrete
name mappings and reachability predicates. Both checked configurations use two
coordinator identities, two requests and at most three terms. `MetadataDistinct`
uses different names; `MetadataSame` uses a shared name. Coordinator identities
are not a two-voter Raft configuration: replicated consensus is abstracted.

The runner performs:

- Four complete safety/liveness checks: both configurations with fingerprint
  polynomials 0 and 1. Their generated/distinct counts must agree. All nine
  actions must have positive successor coverage.
- Six isolated negative controls, each with its own green baseline, intended
  failing mutant and green restored run: replace the ordered barrier with an
  applied committed-prefix read in both name configurations; remove the planning
  term fence in both; acknowledge by index alone; remove apply fairness.
- Three positive reachability witnesses using deliberately false invariants:
  two requests succeed, a write commits after timing out while uncommitted, and
  an uncommitted write's index is consumed by a different entry. These runs must
  violate only the named witness invariant while retaining all safety checks.
- Four runner controls: empty output despite exit zero, missing completion,
  an unfinished exploration queue, and missing temporal checking must fail.

Every run retains exact TLA+ sources, configuration, command, source checksums,
exit status, final statistics and full TLC log, including counterexample states.
Timeouts, parsing/evaluation failures, missing coverage and unexpected assertions
fail the gate. TLC state files are temporary; no checkpoint is reused. The final
`summary.json` is written only after the whole inventory passes.

TLC uses 64-bit state fingerprints. Two polynomial runs and their count agreement
are additional evidence, not a proof that no collision occurred. Logs retain
TLC's probability estimates. There are no state/action constraints, symmetry
reductions, random simulations or search-depth cutoffs. The finite constants
still limit the checked instances. Deadlock checking is disabled because exhausted
finite workloads may stutter; the explicit temporal property checks draining
under its stated fairness assumptions.

See [METADATA-PLANNING.md](../../docs/METADATA-PLANNING.md) for the proof argument,
source mapping, counterexamples and open refinement obligations. This increment
does not close roadmap issues #9, #11 or #14.
