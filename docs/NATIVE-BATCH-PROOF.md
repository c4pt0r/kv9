# Native batch atomic observation and retry refinement

The new TLA+/TLAPS model covers one selected native BatchPut and one selected
BatchGet, interleaved with arbitrary atomic point or batch commands. Its source
correspondence targets the API at `e52e72b4d1020eb078a818f71854d8c179f3dd27`;
the same handler/SDK implementation remains in the accepted native process
candidate `5cc98617b7e299e9f0fc3f46c65ef80ed4182c8a`. No database algorithm or
running server changes in this increment.

This is a parameterized safety proof under the premises below. It is not a
proof of Raft itself, the whole Rust runtime, network availability, stream
resource accounting, or every remaining #50 proof obligation. The strict local
proof/control gate passed with all 479 obligations freshly proved, including
the imported mutation and retry modules.

## Model and claims

`RawMutation` represents physical keys (including CF/mode/keyspace identity) and
ordered mutation sequences. Zero denotes absence; every present byte value,
including an empty byte string, has a nonzero abstract identity. The newly
proved algebra establishes:

- A key absent from the complete mutation sequence keeps its original value.
- At any key present in the sequence, its last pair determines the final value.
- A positional projection preserves duplicate positions and distinguishes
  absence from each present value.

`NativeBatch` refines the existing `ClientRetry` transition system, with a
complete immutable write vector attached to every dispatch. Refusals retire
attempts only when their past and future effects are excluded. Unknown outcomes
are terminal for client dispatch but do not prevent a late server effect.
Each attempt can have at most one lower-layer effect. The proof establishes
one effectful attempt per logical write and preserves the original absolute
dispatch budget and terminal no-replay rule.

The atomic write action stores a before-image and the complete ordered
after-image; both are ghost evidence of that effect. Subsequent background
commands may overwrite the live store, so the after-image is not claimed to
remain current. System-key preservation follows from Raw/System classification
and the freshly proved imported mutation algebra.

The read first enters a waiting phase, then captures one established view.
It collects items individually while the live store continues to change.
The inductive prefix invariant binds every collected position to the same
view, and successful publication requires the complete requested length.
Failed partial collection exposes no successful vector. This proves a
successful read is one ordered projection, including duplicates.

The model accepts arbitrary nonempty finite target vectors; runtime item/byte
bounds are not used to make the safety proof hold. `NBBgLimit` is a finite
exploration horizon for intervening commands, not an implementation bound.
The parameterized result holds for every natural horizon and every supplied
set of finite background commands, including multi-key atomic writes and
singleton put/delete commands. Any finite interference prefix fits some such
horizon. No termination or infinite-run liveness theorem is claimed.

## Premises and source correspondence

| Premise / obligation | Actual source | Meaning in the model |
| --- | --- | --- |
| Complete ordered request and immutable retry payload | `crates/server/src/client.rs` BatchGet/BatchPut calls; `point_wire.rs` dispatch; `grpc.rs::raw_batch_get/raw_batch_put` | Target vectors are fixed constants; dispatch uses that exact ordered vector |
| Quorum authority and context check use the established read view | `runtime.rs::raw_batch_get -> established_read -> read_view_after_barrier -> check_read_view`; `crates/txn/src/raw.rs::batch_get` | Capture obtains one committed state after read invocation; every item uses the captured view |
| Encoded keys preserve CF/mode/keyspace separation | `crates/txn/src/raw.rs::plan_batch_put` and Raw key encoding | Raw target keys are disjoint from System keys; encoding correspondence remains a premise |
| Complete ordered application is atomic and epoch-fenced | `runtime.rs::prepare_raw_write` BatchPut arm; `Command::fenced_write_from_batch`; positioned engine apply | A valid effect applies the complete fold once; Raft commitment, fencing verdicts and engine atomicity are lower-layer premises |
| Success is this effect's exact receipt | `runtime.rs::finish_async_proposal_loop -> settle_proposal`; `grpc.rs::applied_response` | Success requires the current attempt's effect and publishes its bound positive term/index |
| Retired attempts cannot apply later | SDK exclusive NotLeader refusal; server typed Replaced settlement | Retired predecessors exclude past and future effects; a timeout is never such proof |
| One server effect per RPC | Existing proposal ownership and exact completion contract | This premise composes with client no-replay; internal server retries require the same proved-zero-effect predecessor rule |

A committed state is the read-authority premise, not a theorem established by
merely assigning a view in TLA+. The exact-position premise must be supplied by
the waiter/receipt implementation, not an applied watermark or an arbitrary
positive pair. The model's receipt binds the selected logical write's effect;
it does not parse protobuf or authenticate an artifact's receipt.

For real-time interpretation, the whole write effect precedes its successful
return, and the read's captured state lies between its invocation and successful
publication. A lost write response permits a late whole effect; it does not
permit partial publication or automatic replay. These claims are conditional
on source correspondence, distinct from the machine-checked abstract transition
proofs.

## Verification and controls

`scripts/check-native-batch-protocol.py` pins the TLC/SANY JAR, TLAPS release,
standard-module hashes, complete imported graph, theorem inventory and exact
obligation counts. It freshly proves `RawMutationProof`, `ClientRetryProof`,
`NativeBatchAlgebraProof` and `NativeBatchProof`; importing an owned theorem is
not sufficient. There are 55 declarations and 479 expected obligations.

The finite corpus uses two fingerprints per configuration, explicit action
coverage, duplicate keys in a non-palindromic read order, another physical
namespace, and both point and multi-key background commands. Witness searches
require successful batches, safe retry followed by success, an effect whose
pre-state is already unknown, and a completed read whose view differs from the
current store. A separate action witness requires a background update while
the read is collecting. The last two witnesses establish separate reachable
behaviors; their predicates do not require both in one trace. Each action
witness has a baseline/restricted/restored control: forbidding the selected
ordering must eliminate that witness, and restoration must recover it.
Counterexamples mutate one rule at a time:

- Expose only the first write pair to the live store.
- Change the ordered command vector.
- Read each item from the changing live store.
- Reverse positional read results.
- Publish an incomplete read vector.
- Acknowledge another applied position.
- Retry an unknown batch.
- Retire an already applied attempt as a refusal.

Each model and proof control requires baseline success, an attributable
counterexample/failed theorem, and restored success with identical sources.
Proof holes and added axioms are tested in every owned proof module, including
the imported algebra and retry modules. Truncated/incomplete model logs and
incorrect proof obligation reports are rejected separately.

```sh
taskset -c 6-31 python3 -B scripts/check-native-batch-protocol.py \
  --jar /tmp/kv9-p0-tools/tla2tools-v1.7.4.jar \
  --tlapm /tmp/kv9-p0-tools/tlapm-1.6.0-pre-20260731/bin/tlapm \
  --output /tmp/kv9-native-batch-protocol-NEW --jobs 2
```

`--model-only` is explicitly finite preflight and cannot yield proof acceptance.
All work is local; no hosted CI is dispatched.

## Accepted local gate

The corrected complete run at
`/tmp/kv9-native-batch-protocol-witness-order` ended with exit 0 and the exact
marker `PASS: accepted 39 finite cases; 49 proof/audit cases`.

| Gate | Result |
| --- | --- |
| Two finite configurations, two fingerprints each | 11,252 and 22,092 distinct states; every queue empty; matching fingerprint counts and all declared actions covered |
| Reachability | Five witnesses, including actual effect-after-unknown and background-update-during-collection action witnesses |
| Witness ordering controls | Two baseline/restricted/restored triples; suppressing the selected ordering eliminates its witness |
| Protocol mutations | Eight model and eight proof baseline/mutant/restored triples, each with its exact intended failure and unchanged restored sources |
| Fresh owned proofs | RawMutationProof 183; ClientRetryProof 66; NativeBatchAlgebraProof 72; NativeBatchProof 158 obligations |
| Imported and root proof audit | Hole and unapproved-axiom baseline/mutant/restored triples in each of the four owned proof modules |
| Output validation | Three corrupted model outputs and twelve corrupted proof outputs rejected |

The 39 finite cases are four positive explorations, five witness searches, six
witness-order controls and 24 protocol controls. The 49 proof/audit cases are
one full baseline, 24 protocol controls and 24 hole/axiom controls. Negative
controls are expected rejections, not additional proved theorems. The four
owned modules contain 55 theorem declarations and 479 obligations in total.

The accepted summary SHA-256 is
`1163937386a5d0a16bbfce527202504f0dd5e20026544b0d2e0a83f21136e9a3`.
It binds 17 exact source/config/gate files, each copied to `source-snapshot`
and checked unchanged at completion. The final Git checkpoint must contain
those same bytes; this was a precommit source-hash-bound proof execution,
not a clean-commit server build. No Rust runtime was changed or rebuilt.

TLC/SANY JAR SHA-256:
`936a262061c914694dfd669a543be24573c45d5aa0ff20a8b96b23d01e050e88`.
TLAPM reports version `4600b24`; its executable SHA-256 is
`1de5983ac393658362fe61882e270b65648fa07945453e971fcf9eeabf5ec579`.
The inventory separately pins standard modules and audits the complete import
graph. Every owned proof runs with `--strict --nofp` and a fresh isolated cache.
Host affinity was CPUs 6-31 with at most two concurrent proof-control groups.

The independent terminal audit at
`/tmp/kv9-native-batch-proof-review/terminal-audit.json` passed on its first
attempt; SHA-256:
`96d9c1006836a23a7591a7f901dd13df492d5ff4baec0c7de0c466951acf6158`.
It checked the exact process exit and every case/control, revalidated all 49
semantic import records and 33 full successful proof trees, and reread 1,258
files / 25,722,402 bytes unchanged. Its `terminal-inventory.json` has SHA-256
`dbf24fb8068a3eed66e20fc081716afd8276b726d71e53e9b8523a57fa29e93e`;
`terminal-verification.json` has SHA-256
`41e42aed8b05d5d0b468e5e6e4dacac14337789c78b423c1f9bd6c63fee76528`.
The five mapped client/wire/handler/runtime/Raw source files were also checked
unchanged against the declared API and accepted process revisions. These
retained local artifacts are identified by hashes, not embedded in Git.

### Retained witness correction

The earlier full run at `/tmp/kv9-native-batch-protocol-first` completed its
then-current gates, but independent review found its state-only late-effect
witness inadequate: its trace applied the write before reporting unknown.
That summary has SHA-256
`34dce6deec91ab171855a5df269c9992fbd3963999dd5f6e57cd8496b771ab76`;
it is not the accepted evidence for effect-after-unknown coverage. The original
review and all traces remain at `/tmp/kv9-native-batch-proof-review`.

The corrected action witness requires the pre-state to be unknown and the
following transition to add the effect. Its restricted control forbids exactly
that ordering and therefore completes without a counterexample. A similarly
controlled action witness establishes background mutation while collecting.
The older completed-old-view state witness is retained under an accurate name;
it does not by itself constrain when the background update happened.

The independent witness audit SHA-256 is
`1b8bc4569cbbcdc649a694b4df5cdeb763f7a87e855ae066bed724ee2a7800ce`.
It checked the exact transition states, both ordering triples, all 39 finite
cases, the fresh baseline and 519 retained files. This witness audit predates
the terminal proof controls and is not their acceptance record.

Earlier model/proof development attempts also remain outside the source tree:
the first finite configuration used invalid inline action-property syntax;
initial proof attempts exposed missing unchanged-state/type premises. Those
drafts were corrected before the final frozen inputs. No earlier unsuccessful
record is rewritten as a successful final run.

This proof does not newly verify message-size accounting, authorization,
protobuf parsing, stream-generation/request-ID correlation, sibling responses,
cancellation/response-buffer ownership, or liveness under fair scheduling.
Those source/refinement obligations remain in #50; existing tests provide
separate evidence. The accepted [native process histories](NATIVE-BATCH-ACCEPTANCE.md),
actual native Chaos matrix and dedicated client-link effects are separate gates.
