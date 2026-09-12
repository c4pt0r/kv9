# Leader lease design proof validation

See the [complete proof and implementation boundary](../LEADER-LEASE-PROOF.md).
This record concerns a proposed algorithm. Production runtime remains selected
`11113f6` / Safe ReadIndex; no Rust lease implementation, performance campaign or
actual lease Chaos Mesh test ran at this stage.

The fresh local acceptance completed with exit 0:

- Eight parameterized TLAPS lemmas, 23 obligations, fresh cache and `--nofp`.
- Exact SANY theorem/import inventory, pinned standard-module hashes, no module
  assumptions or proof holes in the accepted source.
- Six faulty lemma variants rejected at their intended obligations: missing
  send-time order, prior-vote fence, voting promise, quorum intersection, apply
  fence and final expiry check. These are failed-proof controls, not generated
  distributed execution histories.
- Two proof-integrity controls and three incomplete-output controls rejected.
- Real-clock containment: unsatisfiable negated conclusion over arbitrary real
  constants. Three faulty variants produce satisfying countermodels: late anchor,
  paused leader clock and missing drift margin. Baseline/restored sources match.

The source-bound runner and original outputs are in
[the evidence archive](original-evidence.tar.gz), with exact per-member hashes
in [inventory.json](inventory.json) and the original [summary](summary.json).
Archive: **151 files, 204,118 bytes**, SHA-256
`89a7da554a69e0dc57550a05e13bebe9b1dcf8b4a3aff711b93d758a5edf7ac5`.
Every member was read back and hashed after publication.

The archive also preserves seven proof-development drafts and the intervening
backend diagnosis. Draft 1 is a declaration syntax error; draft 2 did not prove
the real-arithmetic obligations; drafts 3–6 retained an unresolved arithmetic
chain, including the diagnosed SMT timeout. Draft 7 passes after explicitly
proving the order-chain lemma. The real-clock theorem is separately checked with
Z3; changing the TLAPS specialization to integers is not presented as a TLAPS
proof over real clocks. No failed draft was overwritten or counted as acceptance.

Reproduce in a new output directory with the retained toolchain:

```sh
taskset -c 6-15,22-31 env PYTHONDONTWRITEBYTECODE=1 PYTHONOPTIMIZE=0 \
  python3 scripts/check-leader-lease-proof.py \
  --tlapm /tmp/kv9-p0-tools/tlapm-1.6.0-pre-20260731/bin/tlapm \
  --jar /tmp/kv9-p0-tools/tla2tools-v1.7.4.jar \
  --output /tmp/kv9-leader-lease-proof-new
```

The acceptance uses TLAPM `4600b24`, SANY/TLC distribution `1.7.4` for semantic
audit, and Z3 `4.8.9`; binary hashes are in the summary. It does not run TLC state
exploration. The real countermodels concern clock inequalities, not full Raft
executions. The hand-written history argument composes these lemmas with the
explicit Raft, grant/recovery, membership and immutable-view premises. Full
transition-system and source refinement proofs remain implementation gates.
