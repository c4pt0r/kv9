# Endpoint writers and route installation

This increment of [#47](https://github.com/c4pt0r/kv9/issues/47) composes the
[catalog CAS](ENDPOINT-MIGRATION.md) with existing registration and local route
installation. It does not expose the public migration API or complete the
changed-endpoint restart protocol.

## Failure boundaries and implementation

An endpoint generation is meaningful only if every writer preserves its ordering.
Previously renewed registration changed an existing node's address without
advancing its endpoint generation. An operator update also left the old consumed
admission usable, allowing its retry to eagerly reinstall the old address. Finally,
a directory synchronization could capture an old snapshot, wait while a newer
local writer installed its committed address, then restore the old transport route.

`change_endpoint` now reads admission state before staging either row and revokes
an existing Pending/Consumed admission in the same batch as the changed endpoint.
An absent admission remains absent. Refusal and exact-last-transition confirmation
stage no writes, preserving any admission issued after the original update.

`refresh_registration_endpoint` preserves the immutable store binding and advances
the shared generation with checked arithmetic when an existing address changes.
It also records the previous address, so registration A -> B -> A cannot make an
old operator CAS eligible again. An unchanged registration address preserves the
version; initial insertion remains generation zero. The runtime validates and
consumes the admission in the same planned transaction. A consumed retry must
still match the currently applied endpoint before installing its route.

Directory synchronization holds the existing local catalog planner mutex from
snapshot capture through route installation. Registration already holds that
mutex through planning, exact commitment and eager route installation. The
successful-registration-response path now uses it too: an applied catalog row
supplies the address; the successfully contacted leader is a bootstrap fallback
only when its row is absent locally. Root seeds retain their bootstrap role.

Raft application on another thread can advance the catalog while a snapshot is
held. The guarantee is that local installed versions do not regress; it is not
instantaneous equality between transport and the newest committed directory.
A subsequent synchronization catches up once updates stop. This mutex is local
to each replica and adds no coordinator or external availability dependency.

## Parameterized proof and composition

`EndpointWriters.tla` models one existing immutable node/store binding. Addresses
are arbitrary positive identifiers, and the positive generation limit is
arbitrary, including `u64::MAX`. State includes the catalog address/version,
admission lifecycle, installed address/version and one held local capture.

| Model event | Implementation boundary |
| --- | --- |
| `EWNewAdmission` | A new authorized admission after absence/revocation |
| `EWChange(a, TRUE, local)` | Validated registration consumes its admission and updates the existing endpoint in one catalog batch |
| `EWChange(a, FALSE, local)` | Authorized operator CAS updates the endpoint and revokes existing admission authority in one batch |
| `EWStart` / `EWInstall` | The local mutex covers capture and installation; an applied directory row also takes precedence over response fallback |
| `EWEager` | Another local planner's committed update and eager installation, serialized against the held capture |
| Remote change | Raft apply advances the directory independently of a held local snapshot |

The inductive invariant bounds installed generation by the directory generation.
A held capture is between them. Equal generations imply equal addresses. Each
action preserves this ordering because local installers share one mutex and each
address change increments the same generation. Therefore no installation reduces
the installed generation, and an unchanged directory version cannot hide an
address change. Operator changes leave admission absent or revoked; registration
requires a matching Pending admission and consumes it atomically.

The 13-declaration, 102-obligation TLAPS proof establishes initialization,
inductive preservation, those transition effects, and conditional convergence.
Once committed routing stops changing, weakly fair capture and installation
advance an obsolete held snapshot to an idle state, then a fresh capture, then
the current installed endpoint. Each stage is preserved or advances and has an
enabled advancing action. Finite liveness instances start behind the directory,
including one with an obsolete held capture. There is no progress promise during
perpetual updates, unavailable quorum, failed application or unfair scheduling.

The directory projection composes with `EndpointCAS`: changed registration is
an authorized competing update; unchanged registration, route installation,
admission issuance and CAS confirmation/refusal are directory stutters. The CAS
proof separately establishes operator preconditions and exact confirmation.
This extension assumes the existing credential checks, immutable store binding,
term-fenced consensus planning and exact applied receipts. `EWEager` collapses a
local planner's already committed update and installation; the split local-change
actions also allow intervening remote applies. Neither abstraction proves Rust
mutex behavior, whole-program refinement or upstream Raft. Generation values in
the transport are logical history labels; the transport stores immutable address
destinations, and ordering is enforced by serialization, not a new wire field.

Unknown-node fallback, startup seed installation, membership changes, response
receipt semantics and advertised-endpoint restart remain outside this one-row
model. The complete migration protocol still needs their composition and rollout
fencing before its public API is enabled.

## Adversarial local gates

Twelve metadata endpoint tests cover atomic revocation, later admission preservation,
registration ABA, immutable binding, overflow, duplicates and original CAS cases.
Runtime regressions exercise exact consensus commits and renewed registration,
reject both revoked tickets and deliberately recreated obsolete consumed state,
and preserve applied-catalog precedence over registration responses.

The delayed-snapshot regression freezes a real capture using channels, then probes
the actual planner mutex to establish the competing writer's order deterministically.
Two live authenticated gRPC sinks prove original-endpoint delivery before the cut
and new-endpoint delivery afterward. Removing the capture lock makes the old
snapshot reinstall the old endpoint; the intended assertion fails after observing
real deliveries there. Scheduling sleeps do not establish the race cut.

Thirteen isolated compiled Rust source controls require exactly one selected
baseline test, its intended assertion failure under one mutation, and a passing
restoration. The protocol gate checks two finite models under two fingerprints,
two fair continuations, seven faults through both TLC and TLAPS, two reachability
witnesses, two semantic-audit controls and ten invalid-output controls. Each proof
uses a fresh cache and the exact declaration/obligation inventory; proof holes,
custom axioms, truncated output and vacuous counterexamples fail acceptance.

```sh
cargo test --locked --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check
python3 scripts/check-endpoint-controls.py --output /tmp/endpoint-controls-new
python3 scripts/check-endpoint-writers-protocol.py \
  --tlapm /path/to/pinned/tlapm --jar /path/to/tla2tools-v1.7.4.jar \
  --output /tmp/endpoint-writers-protocol-new
```

Real local MinIO and the existing 19-window Chaos Mesh matrix provide regression
coverage. They do not replace the required distinct-address migration Chaos cell,
which must use the forthcoming production API with retained PVC/incarnation,
unavailable old endpoint, exact receipts and complete histories. Single-host Kind
tests do not establish cross-host failure isolation. #47 and #42/P0 remain open.

GitHub CI remains manual-only for pre-release or key-milestone acceptance after
the corresponding local gates pass. Routine development runs these gates locally.

## Accepted local increment

Clean revision `bc5bbd2e8c73c4e811b720f17ab74b0ff7e714f4` passed 540 workspace
tests with zero failures and 22 ignored tests, warnings-denied Clippy, formatting,
Python syntax and workflow validation. All six executable E2E paths and the real
MinIO three-replica failover/checkpoint/recovery path passed. Thirteen compiled
source controls passed baseline, intended failure and restoration.

Independent source binding accepted 29 TLC cases, 28 proof/audit cases and 19
positive fresh-cache TLAPS runs with exactly 13 declarations and 102 obligations.
The two finite instances explored 296 and 2,170 distinct states under each of two
fingerprints. The first full gate attempt matched the wrong failed obligation for
the unlocked-writer fault; the corrected second attempt passed. The accepted third
attempt additionally makes absent admission remain absent, matching the physical
implementation. All earlier attempts and their sources are retained.

Actual local Chaos Mesh acceptance passed the existing 19-window matrix. Independent
audits accepted the complete 4,443-operation CLI history and 1,242-operation
persistent-client history, formation/recovery evidence and corrupted-evidence
controls. The database served after the test collector was removed. Image
`sha256:90f0d7a9de89baf2e4455c6813937703a8f246b6e5f1422c1b7231c3fdd251bb`
contains the retained database executable, whose SHA-256 is
`0ee03532859f5b02a268ecd30b8815e7aff014e080d4ba0c241a7074bf83d2a2`.

Archive: `target/correctness-evidence/2026-09-09-bc5bbd2-endpoint-writers.tar.gz`,
122,683,578 bytes, SHA-256
`2c617348eea9d25fbbd7fa7c0ecf86100789151de7a2d75d8fbdfcd5da3277c9`.
All 3,426 entries independently verify. It retains exact source, the executable,
proofs, controls, original failures, successful logs, Chaos scenes, complete
histories and independent audits. Root-trust and quickstart remove their data
directories on success; their complete process logs are retained.

No GitHub workflow was dispatched. This acceptance completes the existing-writer
integration foundation, not the public migration API, changed-endpoint restart,
rollout fencing or dedicated migration Chaos cell. #47 and #42/P0 remain open;
the separate intermittent initial gRPC apply timeout in #48 is still unexplained.
