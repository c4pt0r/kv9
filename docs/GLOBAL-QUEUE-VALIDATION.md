# Global queue polling: local correctness and recovery

Candidate [3338650](https://github.com/c4pt0r/kv9/commit/33386507d0c068d0243451d44c70b0110e314b4e)
sets `.global_queue_interval(8)` beside the existing two-worker/event-interval-eight
RPC executor configuration. It targets remote Raft read/apply completion wakeups
under sustained local task traffic. The [source rationale](https://github.com/c4pt0r/kv9/blob/33386507d0c068d0243451d44c70b0110e314b4e/docs/GLOBAL-QUEUE-POLLING.md)
distinguishes this hypothesis from the preceding instrumented diagnosis.

Default Raft/Server qualification passes **435 tests/doctests**, with **one
existing ignored** test. Formatting and all-target Clippy pass with warnings
denied. The server is compiled with default features: neither the optional
read-stage observer nor testing fault seams are present. No worker, executor,
transport queue or service has been added.

## Protocol and source correspondence

The core protocol is unchanged from the diagnostic base: exact unique contexts,
sealed read groups, fresh Safe ReadIndex, first exact confirmation, successful
whole-pump completion, unified apply coverage, read-view checks, durable write
acknowledgements, original deadlines and cancellation/reservation ownership.
The opt-in observer is absent from the timed binary. The only added runtime
behavior is the scheduler polling configuration in `NodeRuntime` construction.

The locked Tokio 1.53.1 implementation checks the global queue first at the
configured task interval and local work first on intervening turns. Eight task
selections is not a wall-clock latency bound. Existing protocol proofs and
their fairness assumptions remain unchanged; no new theorem about Tokio or
the full Rust implementation is claimed. The industrial core implementation,
complete Chaos Mesh matrix and independent host-failure gates remain open.

## Original retained build

All **627** declared source files equal the checked source snapshot and clean
commit. The shared build-cache transaction invalidates first-party artifacts
before building and records **11** first rebuilt units / **20** observations.
Actual Cargo records independently confirm empty feature lists for the server,
Raft, server library and engine, release optimization and correct source paths.
Rust is 1.94.0 (`4a4ef493e3a1488c6e321570238084b38948f6db`), LLVM 21.1.8.

| Original artifact | SHA256 |
| --- | --- |
| Server | `052c8959115af73704bd6b07f4ff05c37fcaa082ce9daf5ef83fa6d2115d9894` |
| Build manifest | `9e828eaab534bbfce4c3917459a7e5c9504425eb9adf1a83a4c769026ef5a61c` |
| Build cache receipt | `797c82ad16e03c29051b3640557e183e787831a49b9aa72a1a717d3f14237826` |
| Same-source workload | `2f1e0a5b897cec2b6099a7f96346dfa6d3f9fc30fa7049db5a3434a5d16ca22a` |
| Workload manifest | `589c08458d624bd6d0c10e906e6d54567b4251fc74db8a4eb10eab27efda5af3` |

## Ordinary process recovery

Five inherited contracts pass before the original ordinary recovery fixture.
Both streaming and unary traffic retain complete atomic batch and overlapping
point histories across actual leader loss and original-directory restart.
The independent reader checks all unknown outcomes, consistency, progress in
each fault/recovery window, source/process identities and six fresh voter drains.

| Transport | Operations | OK | Unknown |
| --- | ---: | ---: | ---: |
| Tonic streaming | 174 | 156 | 18 |
| Tonic unary | 189 | 174 | 15 |
| Total | 363 | 330 | 33 |

Unknown responses are retained in the full histories, not silently counted as
success. Both independent history checkers accept their complete inputs. This
is ordinary process recovery; it is not actual candidate Chaos Mesh acceptance,
independent-machine failure acceptance or a proof of the full implementation.

Source qualification terminates in session **20559**, exit 0 (`b7b3a0`);
original release **13411/0** (`8b87ad`) and readback `854875` pass. Recovery
contracts `c74a1f`, runtime **85740/0** (`7c855f`) and independent audit `f8fc8a`
all pass. Original artifacts and first outcomes remain retained. No hosted CI
workflow was dispatched.
