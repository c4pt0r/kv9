# Membership refusal and restart readiness

The root-trust job in [CI 34347685870](https://github.com/c4pt0r/kv9/actions/runs/34347685870/job/102453081986)
failed at `38e7666` when AdmitNode used an obsolete leader after a store restart.
The server returned structured NotLeader metadata, but the blocking membership
client reduced it to prose. The fixture issued one call to its sampled leader.
The same scene separately contains a fresh replacement disk's Raft panic.
The later [#42 acceptance record](REPLACEMENT-ACCEPTANCE.md) covers that defect;
the historical routing increment below did not resolve it.

## Refusal contract

AdmitNode and PromoteNode preserve only the exclusive NotLeader wire result as
`Error::NotLeader`; their CLI renders `not_leader=true leader_node_id=<id|unknown>`.
They reuse the persistent client's existing strict status classifier. Duplicated,
malformed, mixed or unknown control metadata, authentication failure, unmarked
status codes and prose do not prove refusal. The blocking clients still perform
one RPC; they do not retry automatically or reinterpret unknown writes.

The runtime's initial follower check now returns the typed error with a leader
hint from one status snapshot, before ticket allocation or a configuration
proposal. A later term/leadership change retains the existing proposal, exact
receipt and uncertainty rules. In AdmitNode, a catalog-planning Noop can precede
a refusal; that does not commit the requested admission or a ticket. Neither
endpoint gains a retry after an ambiguous configuration or catalog result.

The local acceptance wrapper rotates only its configured voter candidates, uses
one 25-second absolute deadline and passes the remaining budget to each CLI
process. It retries only an exclusive typed refusal with empty stdout and exit 1.
Timeout, partial output and other errors stop immediately. A timeout can leave
server work outstanding; the wrapper does not replay it. Successful ticket output
belongs to the caller and is redacted from routing diagnostics. No new routing
service or mandatory node is introduced.

This instantiates the refusal-only transition of the existing
[client protocol](../proofs/tlaps/client/ClientRetryProof.tla): a target mutation
can be attempted again only after its previous attempt was known not to have
committed. The mapping depends on the server's truthful typed refusal contract;
it is not a new whole-binary proof of membership reconfiguration. Ambiguous
outcomes remain outside the retry transition. The broader #42 receive/incarnation
protocol has separate proof obligations.

## Restart evidence

The first local run after the routing change stopped correctly on an unmarked
MetaNotReady result. Its replica was still Joining: the fixture had mistaken an
old process's Serving file for the restarted process's status. Readiness now
requires the expected child to be alive and one status snapshot to contain that
PID, Serving and an empty fatal field. Missing or duplicated required fields
refuse readiness. Original voters, restarted stores, learner catch-up and
promotion use this check. It does not promise that leadership cannot change
between a status observation and a later RPC.

Retained evidence from development on 2026-09-09:

- `/tmp/kv9-pump-root-trust-failure`: the original failed hosted scene.
- `/tmp/kv9-root-e2e.Uezp7j`: the failed local run demonstrating stale restart
  status and a terminal MetaNotReady outcome; it was not retried as NotLeader.
- `/tmp/kv9-root-e2e.RRkQ3b`: the subsequent local executable run completed all
  script steps. Each of its three membership calls actually received a follower
  refusal and then a successful receipt on the configured next voter.

The latter directory still contains the **known #42 replacement panic** in
`n4-replacement.log`. Therefore a successful script exit here establishes the
routing/restart-readiness regression, not complete identity-rejection acceptance.
The corrected fixture now requires the replacement to remain alive and positively
demonstrate its rejection; absence of Serving is insufficient. Its later acceptance
is recorded separately in [REPLACEMENT-ACCEPTANCE.md](REPLACEMENT-ACCEPTANCE.md).

Validation passed 503 workspace/doc tests (22 explicit integration tests remain
ignored by that command), denied-warning Clippy, formatting and workflow lint.
Real gRPC tests cover known/unknown leader hints and authentication failure;
strict status controls reject ambiguous metadata. Six Python tests exercise the
actual wrapper, deadline propagation, ticket redaction, current/dead/stale process
status, and an isolated missing-PID-check control. Four isolated Rust source
controls each compile, select one test, fail the intended assertion and pass after
restoration. Their artifacts are `/tmp/kv9-root-admin-controls-1`. CI runs these
controls and retains their sources and logs; the exact pushed revision still
requires its hosted gates.
