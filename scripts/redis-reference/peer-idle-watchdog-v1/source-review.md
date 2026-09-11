# Formatted idle-watchdog source and control review

No concrete implementation or control-design blocker was found. This read-only review covers the final formatted queue/test diff from 6707bcc, the five new controls, and PEER-IDLE-WATCHDOG.md. Root's full Raft test session 58483 and later control results are not certified here; no Cargo, tests, fixtures, controls or runtime were executed by this reviewer. Only this note was written.

The production delta removes only the enqueue-to-changed notification. Empty-to-nonempty admission still sets P, takes Body's waker under the mutex, and wakes it after unlocking. Closure/invalidation/outage notifications and all queue/token/body/cooperation operations remain. Session::stalled enables lifecycle Notify before the predicate, reads ownership/P and captures now while locked, uses P+B or locked-now+B, and retains that Sleep inside one select suspension. It preserves both the direct overdue-P check and the post-timer current-P check. An empty timeout merely loops. No failed predicate/result influences Raft state or authorizes delivery.

The document correctly distinguishes the conditional deadline argument from hard real-time or whole-system proof. A first enqueue e after an empty observation t satisfies t+B<=e+B; delayed timer registration cannot replace that selected deadline with a later one. Later valid progress, or drain then new enqueue, moves the relevant timestamp later. Current-state revalidation prevents an old selected timer from expiring refreshed work. Assumptions remain monotonic time, functioning timers, one live watchdog waiter, eventual scheduling/lock access, finite local operations and the existing best-effort transport contract. The claimed removed edge is one explicit notification per empty-to-nonempty burst, subject to coalescing; no measured cause of 6707's regression or improvement is established. Periodic idle timer work and other RPC/body/lifecycle wakes remain.

## Test effectiveness

The formatted file contains 20 component tests: 15 prior tests remain, the old immediate-owner-wake/backdated-after-idle test is replaced by five tests, and the existing valid-dequeue test is strengthened. The new Body/owner test uses distinct counters across two genuine empty transitions, verifies positive Body wakes and zero direct owner wake before the timer can expire. Its short observation precondition has a distinct failure marker, so host suspension is not misreported as the targeted Notify mutant.

Both real backlog variants retain an unpolled Body and compare actual P against before/after enqueue timestamps. The after-idle variant first polls the SAME watchdog to Pending while empty, then waits 150ms and enqueues without changing P. Its outer timeout is actual P+B+1s, and early completion before P+B also fails. In the missing-timer mutant, the Sleep branch is disabled when that first select is entered with P=None. Select's disabled-branch state persists across polls; later polling by timeout cannot reenable it, and no producer Notify is available. The named deadline expectation therefore fails. This specifically checks recovery from a queued workload invisible to Body polling, not merely that an idle future remains Pending. The one-second allowance is a finite test scheduling allowance, not a proof of an exact wall-clock deadline.

The persistent-idle test holds the SAME future for 6.4s, manually polling every 20ms, always requires Pending, and additionally requires >=2 counted wakes while no producer, closure, outage or other lifecycle event occurs. On the reviewed path those wakes come from its timer. An absent timer, or a timer recreated at now+B on each ordinary poll, cannot satisfy that count. False idle expiry fails the earlier reconnect marker. The original no-timer implementation would have remained Pending yet failed the positive wake requirement. This is a concrete observation of two checks, not an exhaustive characterization of all possible malformed timer implementations. Severe scheduling delay can cause this finite observation to fail and must be retained, not automatically classified as a product regression.

The lifecycle test arms an idle timer before each of sender, receiver and Body closure, and separately requires a notification plus immediate terminal polling. Manual polling alone cannot hide a missing closure wake. The existing current-P test now lets its already-armed old timer become ready before valid dequeue wins the queue lock. It then requires the same watchdog to remain Pending under the refreshed timestamp. Its artificial initial P is assigned while the queue is already nonempty, before watchdog arming; it does not recreate the invalid e<t idle history. New semantic assertions copy state out of guards before panicking, avoiding the previously retained poison/destructor-abort failure mode.

## Compiled control mapping (design review only)

The runner retains the original nine controls and adds exactly these five. Each mutant changes a unique source anchor and runs one exact test. The producer-notification injection is inside the newly_pending block, under the queue mutex, so it is an edge-presence control rather than a byte-for-byte restoration of the old unlocked notification placement. Its selected atomic counting waker does not reenter the mutex; the required failure specifically identifies the extra owner wake.

| Control | Exact selected test (under grpc::direct_body::tests::) | Required failure marker |
|---|---|---|
| producer-notifies-idle-watchdog | `producer_wakes_body_but_not_idle_watchdog_across_empty_transitions` | `producer woke the independent idle watchdog` |
| idle-watchdog-has-no-timer | `real_unpolled_backlog_after_idle_arming_expires_without_producer_notify` | `real unpolled backlog missed its unchanged progress deadline` |
| idle-timer-declares-empty-queue-stalled | `persistent_idle_watchdog_checks_twice_despite_repeated_polls_without_reconnect` | `healthy persistent idle watchdog requested reconnect` |
| sender-close-omits-watchdog-notification | `idle_sender_receiver_and_body_closure_notify_without_waiting_for_timer` | `lifecycle closure lost its prompt watchdog notification: sender` |
| old-timer-ignores-current-backlog-progress | `valid_batch_progress_resets_armed_budget_and_empty_queue_clears_it` | `an already-armed old deadline ignored real dequeue progress` |

The runner rejects ambiguous mutation anchors, zero/multiple selected tests, compiler failure, wrong assertion, abnormal termination and failure of either baseline or restored source. It requires mutant exit 101 plus the marker and `0 passed; 1 failed;`; successful phases require one passed test and exit 0. It binds/restores GRPC, BODY, BODY_TESTS and TCP before every phase, checks their actual hashes afterward, and verifies the root copies remain unchanged. Those four explicit source bindings are not a full dependency-tree proof. Controls compile in a private copied source tree with the selected Cargo target. No control result is presumed from this source review.

Retained original control names: `missing-route-notification`, `address-reuse-admits-old-generation`, `replace-live-worker-on-every-send`, `tcp-drop-retains-listener`, `tcp-retains-old-connection`, `stale-body-polls-replacement-queue`, `stale-drop-invalidates-replacement`, `enqueue-postpones-stall-deadline`, `ready-body-restores-cooperative-budget`.

## Reviewed source hashes

- crates/raft/src/grpc/direct_body.rs: `1c758a1b0e41ec5de11f06e919b50f962e2072fb463cc50e12523dfa0fefb0fa`
- crates/raft/src/grpc/direct_body/tests.rs: `e21f879f5b62737b4c331644d8e56a0a14a14c3a009b84ab204b944701133389`
- crates/raft/src/grpc.rs: `94678473f675a3e2f352f280511986bec4b6c370551718f7ae1523d3b498cb37`
- crates/raft/src/transport.rs: `090642e0b487142ef32b3cc9f9d35fcec26fef9d95ba7942664a9e0db9419f58`
- scripts/check-route-controls.py: `2dc93978f4a291694958c8ae9c1c3d59ea315c6003d439e247e15a1e89f6da4e`
- docs/PEER-IDLE-WATCHDOG.md: `b567bb228b26d98326db8c16b5e9ebbcd375ffd442cead0760af3560a234678a`

## Status and documentation supplement

Root subsequently reported session 58483 terminal 0: 197 library + 19 integration + 12 documentation tests = 228; Clippy session 26451 terminal 0. Root reported control session 74063 still live with 11/14 triples completed. These are root-reported results pending retained terminal-log readback; this reviewer has not rerun or independently accepted their execution.

The appended Local source gates section of PEER-IDLE-WATCHDOG.md and the full follow-on delta in DIRECT-PEER-BODY.md were read. The former's five control descriptions match the actual mutations, with execution results attributed as above. The latter now explicitly separates original 6707 test/control results and rejection from follow-on acceptance, replaces the obsolete producer-notification statement, and links the idle deadline argument. No material contradictory scope claim was found.

- Current PEER-IDLE-WATCHDOG.md SHA-256: `7a831e156bb0b0e4ddda609eb195c5a22fbc006aaaa27cc5c33cf3bf52ea67f1`.
- Current DIRECT-PEER-BODY.md SHA-256: `f746d7feb829736d1b3194da7da39f92a775308fc136a49df9ab26d38656b34e`.
- Pre-supplement review SHA-256: `b066679a9158b017db8cb22d374a02ed7b479a272f6bc576e301d511e208b512` (original bytes preserved).

## Independent terminal evidence readback

Readback accepted all 14 triples / 42 retained compiled executions. This section supplements the prior source review without changing its 9305-byte prefix, SHA-256 `885d4b299f54af267882499f2f7a5cbe645680666b20a753085ac3ddc0219a13`. No Cargo, runtime, source mutation or test rerun occurred.

The exact control manifest is `/tmp/kv9-peer-idle-watchdog-route-controls-first/manifest.json`, SHA-256 `4fe438eec1dabf10ee51be45529d3191f2d1984b6c5e27397775dc6b66a1861c`. All 14 case names, selected tests, assertion markers and source paths match the reviewed runner. Every retained mutant was independently reconstructed from its unique source replacement and compared byte-for-byte. Each phase's four-source map matches the retained originals plus only that phase's intended mutant; this includes the unchanged split-out test source in all 42 maps (168 bindings). Current production/test/control hashes match the validation record and the retained original files. Current source is clean at `f62c08ed9dc59bf56336a64a24c76c2b775f634e`.

All 42 logs select exactly the stated one test. Each baseline/restored log reports one passed, zero failed/ignored; every mutant log reports its named test FAILED, the required panic marker, zero passed and one failed, with the recorded 0/101/0 sequence. No compile failure, signal abort or destructor-panic substitute was accepted. The root control summary records 14 completed triples and terminal session 74063 exit 0. The SHA-256 of the canonical JSON map of all 42 log hashes is `637d2bcdd89a382b4665f23ed4deedafb09688497c338332ac3fe6d02450f244`: keys are `<control-name>/<phase>.log`, values their SHA-256, serialized with sorted keys and separators comma/colon without whitespace. Paths and key inventory are recoverable from the bound manifest.

Validation record `/tmp/kv9-peer-idle-watchdog-validation-first/result.json` SHA-256 `eb83fc8c200ba96952b419bcf520bc67c2973dffcb567b8e92fb96bd89806e95` binds the three terminal gate logs, whose bytes were read back. The Raft log contains pass counts 197/1/13/5/12, total 228 with zero failures; its library/integration/doc grouping is 197/19/12. Clippy's retained log ends in successful dev-profile completion with no warning/error diagnostic. Recorded sessions 58483 and 26451 both exited 0; all 14 control triples are now terminal, replacing the earlier pending status. These are retained execution evidence plus parent-confirmed process status, not independently rerun commands.

The final PEER-IDLE-WATCHDOG.md append states 14 triples / 42 compiled executions, exact 0/101/0 exits and named assertion rejection. Those statements match the checked logs and manifest. Its current SHA-256 is `7a831e156bb0b0e4ddda609eb195c5a22fbc006aaaa27cc5c33cf3bf52ea67f1`. The unchanged DIRECT-PEER-BODY.md historical/follow-on distinction remains as previously reviewed. These local gates do not establish process E2E, Chaos or a throughput gain for the follow-on candidate; the release build is outside this readback.
