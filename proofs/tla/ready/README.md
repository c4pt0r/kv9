# Ready persistence and publication

`ReadyPublication.tla` models the per-peer ordering between Raft storage,
`advance_append`, publication to the driver, durable application, logical
acknowledgement and crash/reopen. TLC and TLAPS use these same transitions.
See [READY-PUBLICATION.md](../../../docs/READY-PUBLICATION.md) for the Rust
mapping, assumptions, theorem scope and conditional progress argument.

Run the finite checker with the pinned TLC jar described in the
[metadata model instructions](../README.md):

```sh
python3 scripts/check-ready-tla.py --jar /path/to/tla2tools.jar \
  --output /tmp/kv9-ready-models
```

The output directory must be new. `Ready2.cfg` and `Ready3.cfg` enumerate index
bounds two and three; these are not voter counts. Each runs with fingerprint
polynomials zero and one, requires matching exploration statistics and nonzero
coverage for all eleven named actions. Every run checks index coverage, stage
constraints and failure publication exclusion. `RPFatalFreeze` is an action
safety property, not a temporal liveness claim.

Three isolated controls each require a passing baseline, a concrete rejection
and an identical restored source:

| Mutation | Required counterexample |
|---|---|
| Skip the late commit sync | Delivered work exceeds the durable commit watermark (`RPCoverage`) |
| Publish on a persistence error | Failed cycle publishes (`RPNoFailedPublication`) |
| Apply after fatal failure | Failed peer advances application (`RPFatalFreeze`) |

The mutations are shared with the TLAPS checker in `scripts/ready_controls.py`.
They change only `ReadyPublication.tla`. Two deliberately false invariants also
require traces reaching a late commit and a crash after positive acknowledgement.
Three output controls reject empty, truncated and unfinished exit-zero runs.
Parser failures and timeouts cannot count as counterexamples or successful checks.
The complete gate has 15 TLC runs and retains copied sources, source/tool hashes,
commands, logs, action coverage, counterexamples and a final summary only on pass.

Finite enumeration is not the parameterized proof. The six theorem declarations
and 75 obligations in `proofs/tlaps/ReadyPublicationProof.tla` establish safety
for every legal positive index bound and every behavior of this model, including
arbitrary failure/restart/stuttering. They do not prove the upstream consensus
algorithm, filesystem semantics, Rust execution or progress.
