# Independent ReadIndex credit review and proof proposal

Reviewed the candidate based on accepted `5ee897a2f58c57bdf17ea1757c96224adf0f0dbb`. No implementation blocker found in the new same-lock admission/wake predicates. This note proposes a bounded proof; it does not claim that proof has run. No repository edits, Cargo, proof execution or runtime work were performed.

The retained focused run has five named credit-test successes and a terminal summary of 185 library tests passed. Root subsequently added two synchronous tests and new controls. The first note-construction check correctly stopped when the current test hash differed from the focused manifest; no file had been written. The 185-test result remains historical, bound to its recorded `f45e18acca20329fc427b98bf46ea9406991da755a75ac03ef3f62c27cd69d26` test file. Current tests and controls are reviewed below, without borrowing that result. Root reports exact-current validation running in session 25535 at `/tmp/kv9-read-group-credit-source-validation-first`.

## Exact source argument

`RaftPeer::read_index` checks fatal state, leader role, current-term commitment and `pending_read_count() < 1` under the same peer mutex as `RawNode::read_index`. A false return submits nothing. The synchronous loop retains one minted context and original start time; asynchronous deferral retains member identities/deadlines. The owner wake predicate permits followers to run admission and receive typed `NotLeader`, while full committed leaders suppress the retained queued-work self-wake.

Pinned raft-rs 0.7.0 provides the authoritative occupancy:

- `read_only.rs::pending_read_count` returns `read_index_queue.len()`. `add_request` appends one previously absent context and seeds its ACK set with self. Caller cancellation cannot modify this queue.
- `raft.rs:2104`, leader `MsgReadIndex`: Safe mode appends a request; the singleton branch instead returns a read state synchronously without a pending entry. The KV9 configuration explicitly selects Safe mode.
- `raft.rs:1857–1865`, heartbeat-response handling: only a matching ACK set satisfying the current quorum advances the prefix through that context.
- `raft.rs:2711–2727`, `Raft::post_conf_change` (declared at line 2667): after changing configuration, the newest pending context is checked against the updated quorum, and a satisfied quorum also calls `read_only.advance`. This is a real release path in addition to heartbeat ACK processing.
- `raft.rs:986–1001`, `reset`: recreates `ReadOnly` with an empty queue. A role-label change alone must not substitute for the actual reset transition.

The gRPC receive handler authenticates sender/root/receive authority, but has no `MsgReadIndex` exclusion. Inbound messages reach upstream `step` outside the local wrapper. Remote requests can therefore exceed one pending entry, and those entries still block local admission. The existing document correctly states a local admission policy, not a universal queue bound. Well-formed, non-Byzantine peer behavior and upstream quorum/context correctness remain explicit premises.

## Small parameterized proof

Add a separate `ReadCredit` extension, retaining `ReadAdmission` as the comparison abstraction. Model authoritative upstream occupancy `p`, positive limit `C` (implementation: one), and events distinguishing local admission, remote admission, upstream advance/reset and cancellation. Do not substitute registry membership or apply-held group count for `p`.

1. **Atomic admission:** a local submission requires leader, current-term commit and `p<C` at the same linearization point as the upstream call. The upstream transition adds at most one entry; therefore local submission has `p'<=C`. A conservative count abstraction allows `p'` in `{p,p+1}`. Fresh context plus non-singleton Safe mode justifies exact `+1`; immediate singleton confirmation needs its own case if proving receipt correspondence. There must be no environment transition between guard and update.
2. **Induction:** prove natural occupancy, no local increase when already full, cancellation leaving occupancy unchanged, prefix advance not increasing it, and reset clearing it. Initial-empty/local-only executions imply `p<=C`. With remote insertion enabled, prove the local action theorem, not the global invariant. Two remote contexts are a valid counterexample to the latter; do not artificially bound remote arrivals to hide it.
3. **Safety projection:** full-credit deferral preserves request phase, submit count, context, budget and evidence, so projects to a stutter of `[RANext]_raVars`. Successful admission projects to `RAPoll`; follower refusal remains the original branch. Credit release cannot manufacture quorum, application or return evidence. If caller cancellation is included, an auxiliary caller-alive flag may project to stutter. Existing ReadAdmission has no positive-budget cancellation/refusal action: silently projecting cancellation to `RATimeout` would be invalid.
4. **Scheduling:** preserve prefix cleanup before attempting credit admission, bounded 64-item inspection, deferred retained-prefix suppression, and the final capacity-aware wake predicate. Inbound ACK processing precedes admission, allowing release and a fresh submission in the same owner turn. Completion publication wakes synchronous retries; their observed-generation-before-predicate pattern remains. Notifications grant no authority and admission always rechecks the peer lock.

This projection is a proof of the admission primitive. It does not establish the mapping from regrouped asynchronous members to a representative quorum context: deferred groups may be regrouped, whereas the old single-invocation model keeps its context fixed. That mapping belongs to the still-open grouped-read composition.

## Progress conditions and deadline wording

Do not inherit `RAStableSuccessProof` merely by strengthening the guard. Its Stage1 enabledness argument assumes a committed leader can submit. A full-credit waiter can stutter forever while weak fairness of the now-disabled nonstuttering poll is vacuous. Require eventual persistent slot availability for the selected request, or an explicit allocation/no-starvation condition under competing local/remote refills. Fair owner service and eventual quorum traffic alone do not rule out another caller repeatedly taking every released slot.

With those conditions, stable leadership/current-term readiness, valid eventual upstream release, continuing owner service and sufficient caller budget support conditional progress. Ordinary heartbeats resend the newest pending context even after all its callers cancel; protocol reset can also clear it. Neither caller lifetime nor elapsed time permits a fabricated refund. The old stable model omits budget ticks and gives no wall-clock success bound.

The synchronous loop checks elapsed time after an unsuccessful admission attempt, and checks successful evidence before elapsed time in later loops. This preexisting ordering preserves the original budget without restarting it, but does not prove that no admission or return can occur after the wall deadline following a delayed wake. Keep the narrower preservation claim. The new queued-deadline test correctly tests expiration while credit remains full, rather than claiming the stronger property.

## Tests and controls

The five original real-trio tests cover: cap/late-context separation; 70 queued cancellations drained in bounded 64+6 inspections while protocol credit survives the last member; positive actual-owner parking and real ACK wakeup before a 30-second tick; confirmation releasing credit before application; and actual transfer/reset with follower refusal and fresh leader reading. Root's two added tests now cover synchronous sharing with a distinct confirmation and the original queued synchronous deadline. Their current execution result remains pending this review.

The updated async control runner has 14 cases: the prior 11 plus bypassed pending credit, credit-blind owner spin and synchronous bypass. The changed per-member-broadcast assertion is appropriate: the cap blocks its second callback, so lost group membership is the first semantic failure before another heartbeat could be emitted. Full copied-input inventories protect the split-out tests in this runner. The older separate synchronous control runner binds RAW/DRIVER/GRPC only; if extended to the new tests, bind the split-out test source explicitly too.

For the formal extension, retain all eight existing ReadAdmission mutation contracts. Add bounded controls that distinguish: bypassing the cap; split guard/update allowing two callers to observe zero; refunding canceled occupancy; retaining credit until application; credit-blind owner self-wake; missing release/allocation progress; and budget growth. A cancellation-refund model must compare accounting to an independent protocol queue rather than mutate its only occupancy variable and call that value authoritative. Use at least two contexts and positive traces reaching full deferral, cancellation, release, late fresh admission, apply-held confirmation, reset and follower refusal. Singleton and configuration-induced release remain useful explicit cases, not exercised by the five new trio scenarios.

Each semantic mutation should fail its named TLC property and intended TLAPS obligation with baseline/mutant/restored source binding. Preserve existing fairness controls; otherwise a strengthened guard can make old liveness controls vacuous. A no-release execution without a fair-release premise is an expected scope counterexample, not an error to exclude silently. No new formal/control execution is claimed here.

Full sealed-group freshness, multi-request scheduling, upstream quorum/term correctness, successful Ready persistence/application, view authorization and Rust refinement remain separate open composition obligations.

## Reviewed hashes

- `crates/raft/src/rawnode.rs`: `97c0046a635bd6f09c0f8deb8371ad6e808996ce69bd6df62efc4e946f759eef`
- `crates/raft/src/driver.rs`: `dcd8bea84fc0b315f21a2fb74f05bf390a7cec387582c64c0d64f7229f3145c5`
- `crates/raft/src/async_read.rs`: `7cae8d4bd986ff60d20a22d3f04dc4d39721f80f68d489a5ea98eefa32eb020f`
- `crates/raft/src/work.rs`: `766fe563ceba1f2e398ce14704b9fc232be3b4fa52b0301783036f95f089164a`
- `crates/raft/src/grpc.rs`: `9083374c5ac34abac7ed8eb5967b1c5fdb693b397e6f6570131e9691e422ac25`
- `crates/raft/src/driver/read_credit_tests.rs`: `a1ceb8b977f1c30d3f7cd44fad3dc2e044a4a77fb6cb03f131fd7eb20b089dcb`
- `scripts/check-async-read-controls.py`: `4edf65bd257936b96016bdec4e4df7b0be0cc69ed49b9b1d6f4b3b15f7491870`
- `scripts/check-read-admission-controls.py`: `9a96788a89c67128153108ca7a2da2d9e36d00f6df33d8ee4725294040b7822d`
- `scripts/read_admission_controls.py`: `0e7eaa517f16ee15eb2ccd86d81f8b739768b0362fabcf649b4098c4d87154d0`
- `scripts/check-read-admission-protocol.py`: `fbe766802fe9f109237aef41426e002c7b23a5e6c3f79f49396062709c662b9b`
- `proofs/tla/read-admission/ReadAdmission.tla`: `538e3aa3c524a5a11a67d7e3076c4b3cf4fa403310731f1d53b9094cb82a7227`
- `proofs/tlaps/read-admission/ReadAdmissionProof.tla`: `0fc9a3fb7aaca030d92078904096346eb437f0351d9ac0536f1a8e9eac6b8e51`
- `proofs/tlaps/read-admission/inventory.json`: `1a8a1b1fb9e23d992544739d93d2f251e11e85e48ceb21827a60caadc025ea6f`
- `docs/READ-INDEX-CREDIT.md`: `c3827335696162ad6e15afc6fc8cf3003d4d9c754ba54b89a657f36873ad8fe1`
- `/home/dongxu/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/raft-0.7.0/src/raft.rs`: `a595cc7420eb83f49546253d004ebc9657e255581db467376a13cf02a383a8eb`
- `/home/dongxu/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/raft-0.7.0/src/read_only.rs`: `0af28746d1bdb6e8741784a23f5cfa791f27ef3d3978774a6d0a94727fa1ec49`
- `/home/dongxu/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/raft-0.7.0/src/raw_node.rs`: `92cbca1fa82288ad8ccecbc74066bb766ea38b5ffce4ed89a0d1e0f174ddb057`
- `/tmp/kv9-read-group-credit-focused-first/manifest.json`: `7d174757c1cb658c95d227e0a86034f2a9a58d8fb9c928140be32e66bf13865e`
- `/tmp/kv9-read-group-credit-focused-first/tests.log`: `6fa30430b290295b5c3308babbdcb9c5a4c3165d0c4d396ff7a2c401e1064e6f`


## Second formal review: ReadCredit model and proof draft

This addendum preserves the preceding bytes exactly (prefix SHA256 `a5ffa982446e0735508df055680e06b61d7e3b7f11b2cfdbb414d6bd61ed2891`). It is read-only source review, with no proof or runtime execution.

The projection argument is sound at its stated abstraction boundary. Full-credit RCPoll leaves raVars unchanged; its other branch invokes the existing RAPoll. RCReadStep and RCReset project to existing actions, while cancellation/other-admission/release project to stuttering. Cancellation is auxiliary rather than being misrepresented as an expired timeout. RCCreditSafety consequently reuses the separately checked ReadAdmission safe-return theorem without claiming a new quorum or Ready proof. Release itself supplies no request receipt or application evidence.

The first draft incorrectly assigned TRUE occupancy to every submitted call while claiming to abstract actual pending_read_count>=1: a singleton returns an immediate ReadState with zero pending occupancy. Root corrected RCPoll to permit either Boolean post-state on submission, and documented the singleton case. That resolves this source-case exclusion. The result is deliberately an overapproximation, not a queue-conservation proof: there is no upstream queue or count to independently validate RCRelease, and remote additions while already occupied are represented as stutters. The model is parameterized by the inherited request/term/budget inputs but fixes the occupancy threshold at one; it proves neither an arbitrary-C queue bound nor universal pending_count<=1.

RCProgressSpec starts in a ready, uncanceled Waiting window and allows only RCPoll or RCRelease plus stuttering. No competitors, role changes, cancellations or deadline expiration occur in that window. WF(RCRelease) frees a persistently blocked slot, and with refill excluded WF(RCPoll) then admits the request. This is a valid, deliberately stronger environment assumption than ordinary owner fairness. It proves eventual admission only, not completed reads, concurrent-caller fairness or latency.

The current RCWaiting explicitly requires positive remaining budget; this addresses the identified expired-initial-state mismatch. RCProgressNext still intentionally excludes budget expiration, so this is a stable unexpired window, not a wall-clock guarantee.

The named effects properties are meaningful under isolated action-definition mutants. Removing occupied/canceled checks from RCPoll leaves the independent consequent of RCAdmissionHasCredit unchanged; Waiting-to-Submitted from such a pre-state violates it. Changing RCCancel to clear TRUE occupancy violates RCCancellationEffects: the mutated action is the antecedent, while the unchanged preservation predicate is false. These proofs are syntactically direct on valid definitions but are not immune to the proposed mutations. Freeze the property definitions during mutation; changing the property along with its action would destroy this control. Require the occupied/canceled pre-state and actual submission/cancel transition in retained counterexamples.

For fairness controls, removing release fairness has an occupied initial witness; removing poll fairness has a free initial witness. Both initial states are admitted by the progress initializer. Preserve those witnesses and the specific temporal failure, rather than merely accepting any failed exploration. State predicates alone cannot detect a bad cancellation transition or distinguish a harmless repeated poll from missing progress; use the existing action-effects and temporal properties accordingly.

ReadCreditProof imports ReadAdmissionProof explicitly, and the inventory lists both proof modules (196 and 69 obligations), both model sources and only the inherited legal-input assumption. The extra stability/no-refill premises are in RCProgressSpec, not hidden ASSUMEs, but still must accompany every success claim. The final strict gate must separately audit/prove those imported inputs; importing their theorem names is not itself fresh validation.

Retained development evidence was read back: dev1 exit10, six of68 obligations failed; dev2 exit0, all69 obligations proved. Dev2 uses earlier model/proof hashes than the current singleton/effects/TLC additions. It is not acceptance of this revised snapshot. Fresh strict inventory/import/hole/control gates remain pending; none were run by this reviewer. Full grouped-read/context/Ready/Rust composition remains open.

Addendum source and retained development hashes:
- `proofs/tla/read-credit/ReadCredit.tla`: `7008f18ce50ad772c4ab97b8da83e03ebca112597c17708fe8443dddbb1eab04`
- `proofs/tla/read-credit/ReadCreditMC.tla`: `afcb6f11173dbdf8cac04e254786bcf4be3731a1d162d20b00ae7aa4c0a8ff13`
- `proofs/tlaps/read-credit/ReadCreditProof.tla`: `75baf0808ec1b70c332c8ea496decedef527ca1efd446c220efebbaee109fd74`
- `proofs/tlaps/read-credit/inventory.json`: `34c8e9ff920f5dfbcc1bd6f698e8953bd9ada93f1fa0bc4bd7f2f7a7657ec9f7`
- `proofs/tla/read-admission/ReadAdmission.tla`: `538e3aa3c524a5a11a67d7e3076c4b3cf4fa403310731f1d53b9094cb82a7227`
- `proofs/tlaps/read-admission/ReadAdmissionProof.tla`: `0fc9a3fb7aaca030d92078904096346eb437f0351d9ac0536f1a8e9eac6b8e51`
- `/tmp/kv9-read-group-credit-proof-dev1/run.json`: `a4ed3fc12fbfabc8a38f6140b6e655a45955b176d0f19ad9fab1c27dbbf95ef9`
- `/tmp/kv9-read-group-credit-proof-dev1/tlaps.log`: `4fec8098221ecb91437276d23f9d07624661ef0dea551a8d9bc5c2e05080b329`
- `/tmp/kv9-read-group-credit-proof-dev2/run.json`: `38f5c6f00d1762f1094e85c12a8b8374b06acd9454061e31aa9a6964a43981f9`
- `/tmp/kv9-read-group-credit-proof-dev2/tlaps.log`: `60fd1c02beea72ffd775fcd69e78961319d399f8c21db28f5bfdda0e8c01aae6`
