# Outbound peer executor: source and ordinary recovery validation

Prototype [36ae89a](https://github.com/c4pt0r/kv9/commit/36ae89a774131b368cf1ed28e95df02ba325c8d4)
passes local server checks and actual retained-binary stream/unary leader-loss
and original-directory restart histories. **No performance comparison has run**
for this prototype. Selected production behavior remains CRC `ca0002c7`.

The public executor retains two workers, event interval eight and adaptive
global scheduling. One additional node-local worker runs the existing outbound
GrpcTransport tasks and connections. Inbound Raft still shares the public
listener/runtime. Original queue/coalescing, generation filtering, cancellation,
connect/progress watchdogs, authentication and consensus guards remain intact.
This adds a worker; it is neither complete network isolation nor affinity
isolation. There is no new per-message task, network port or cluster service.

The [source contract](https://github.com/c4pt0r/kv9/blob/36ae89a774131b368cf1ed28e95df02ba325c8d4/docs/PEER-EXECUTOR-ISOLATION.md)
maps unchanged application transitions under different scheduling and describes
construction/drop ownership. This conditional correspondence does not prove
Tokio, whole-Rust refinement or all startup failure paths. A successful process
exit does not independently prove field drop order or inject executor-constructor
failure. Core proof composition and industrial availability gates remain open.

## Local evidence

- Source gate session 34497 exits zero (`6e564a`): formatting, 224 default and
  234 experimental server tests/doctests, and all-target Clippy. The suites
  overlap and each retains one pre-existing ignored test.
- Original default release session 84681 exits zero (`1c018f`). Independent
  readback exits zero (`6871c5`), binding 595 source files, 11 fresh first-party
  units and 20 artifact rows after retained-cache invalidation.
- Five preserved process-preparation contracts pass. The original process
  fixture session 74225 exits zero (`fe77f9`). The unchanged independent auditor
  passes (`c5148a`), checking both complete atomic histories, point/batch overlap,
  all unknown outcomes, original executable identities and fresh drains.
- Stream records 179 calls, 163 OK and 16 unknown; unary records 177 calls,
  165 OK and 12 unknown. Total: **356 calls, 328 OK and 28 unknown**. Unknown
  writes are retained without blind replay. All five server/two client lifetimes
  exit, with six fresh voter drains.

This is ordinary process leader-loss/restart recovery. Actual Chaos Mesh,
host/power loss, full-workspace qualification and performance acceptance have
not run for this exact source. Unit tests using their own runtime do not alone
exercise the production constructor; the retained-binary process gate does.

| Artifact | SHA-256 |
| --- | --- |
| Original server | `94a0971ca57d884b3eb63aca29c8805b7f857becfa73dfe508966e82460c7d81` |
| Original build manifest | `6af4a691b4976695dbaceaf88d0bcc1ac6c33b22fb401191f3c60cd429c7dd2d` |
| Source tree | `e2ac84d839604b320652e2be4173355b849fade839338b98af7155d185a01f64` |
| Cache receipt | `23ad1eda9319f9f6e7967c0c74016e1ea64e9b925959196b689fee1bb5a914e4` |
| Frozen process preparation | `570c7517302488f53caa8bcac9a42a148c6441e270e49e0475a16607f2c9a549` |

## Next acceptance path

Screen the exact uninstrumented prototype against selected CRC and same-run
Redis at c1/c64 GET and mixed traffic, preserving complete forward/reverse
repetitions, separate GET/PUT means/tails, CPU identity and storage guards.
If promising, compare a shared three-worker/event8 control to distinguish
placement from worker-count effects. Incoming peer handling remains a separate
boundary; do not infer full isolation from an outbound result.

Only useful candidates advance to full point/batch checks, applicable proof
mapping and actual exact-source Chaos Mesh. Preserve fresh Safe ReadIndex,
sealed groups, successful pump/apply/view fences and durable acknowledgements.
Keep original evidence while preparing enough retention space; do not lower the
acceptance floor. The [event-one regression](RPC-EVENT-ONE-PERFORMANCE.md) stays
rejected. Dynamic multi-Raft and automatic splits remain after the read milestone.

The [compact evidence bundle](peer-executor-isolation-v1/README.md) retains
original source/release checks and complete ordinary histories. It is integrity
evidence, not a standalone full runtime audit. No hosted CI was dispatched.
