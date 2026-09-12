# C1 quorum observer: accepted capture, incomplete context timing

All four original cohorts passed runtime and independent readback: 562,636 measured GETs succeeded with one attempt, no dropped slots, and 612,312 successful all-phase calls. All six trace prefixes lost selected observations to observer contention (1,083 in total). **No complete-context RTT attribution is available.** This is an instrumentation diagnostic, not a performance promotion.

The fixed order was control → trace → trace → control, one c1 pure-GET cohort for each role in each order. Every cohort used 5 seconds, 4,096 keys plus sentinel, 128-byte values, 128 warmup calls, seed 71, the original closed-loop v3 streaming client, three ordinary voters and tmpfs WAL. Client/server/helper CPU placement and storage/lifecycle guards were unchanged. No Redis trial, loaded c64 trial, fault or new optimization comparison is included.

## All four client results

Whole-call latency is the measured client call, including the original post-cutoff completion. Each cohort has one such completion; none is dropped or removed. Quantiles below are raw histogram bucket intervals, not averaged percentiles.

| Cohort | GET successes / attempts | GET/s | Mean µs | p50 µs | p95 µs | p99 µs | All-phase calls |
|---|---|---|---|---|---|---|---|
| control-c1-r0 | 142,470 / 142,470 | 28493.848 | 34.984155 | 34.304–34.815 | 39.424–39.935 | 44.544–45.055 | 154,889 |
| trace-c1-r0 | 139,985 / 139,985 | 27996.907 | 35.601629 | 34.816–35.327 | 39.936–40.447 | 45.056–45.567 | 152,404 |
| trace-c1-r1 | 138,390 / 138,390 | 27677.949 | 36.018606 | 35.328–35.839 | 40.448–40.959 | 45.056–45.567 | 150,809 |
| control-c1-r1 | 141,791 / 141,791 | 28358.170 | 35.152992 | 34.304–34.815 | 39.424–39.935 | 44.544–45.055 | 154,210 |

Per cohort, initialization contains 4,097 BatchGet and 4,097 BatchPut calls, warmup 128 point GETs, and verification 4,097 BatchGets. Measured point PUT populations are zero. These setup/verification batches retain their original labels. All outcome, attempt and API/item populations remain in `summary.json` and the four original reports. Final scans retained 4,097 deterministic values with nonce zero and unchanged sentinel; no complete history or linearizability proof is inferred.

| Order | Trace GET-rate change | Trace whole-call mean change |
|---|---|---|
| control→trace | -1.7440% | +1.7650% |
| trace→control | -2.3987% | +2.4624% |

Pooling by successful calls / elapsed time gives 28426.009 control and 27837.428 trace GET/s (-2.0706%). Count-weighted whole-call means are 35.068372 and 35.808923 µs. No pooled percentile is computed. Both trace p99 intervals are 45.056–45.567 µs versus 44.544–45.055 µs for control.

This measures the combined diagnostic build, including both quorum-message observation and the six read-stage histograms. It does not isolate the cost of the raw trace collector, establish statistical significance, or remove shared-host/order/cache effects. Source and feature identities are listed below.

## Six independently bound process prefixes

Node 2 was the observed leader, term 1, in both trace cohorts. Identities below are independently joined to launch/cleanup, listener, status and exporter evidence. Counts belong to each local prefix, including setup, verification and post-client activity; they are not measurement-window event counts.

| Cohort / node | PID | Role | Recorded events | Contended losses | Other losses | Local tickets | Valid partial spans | Complete contexts |
|---|---|---|---|---|---|---|---|---|
| trace-c1-r0 / 1 | 500321 | follower | 4632 | 10 | 0 | 1160 | 3457 | 0 |
| trace-c1-r0 / 2 | 500326 | leader | 10878 | 530 | 0 | 2224 | 6276 | 0 |
| trace-c1-r0 / 3 | 500331 | follower | 4630 | 10 | 0 | 1160 | 3459 | 0 |
| trace-c1-r1 / 1 | 501741 | follower | 4592 | 10 | 0 | 1150 | 3429 | 0 |
| trace-c1-r1 / 2 | 501746 | leader | 10788 | 516 | 0 | 2204 | 6207 | 0 |
| trace-c1-r1 / 3 | 501751 | follower | 4593 | 7 | 0 | 1150 | 3433 | 0 |

The observer recorded 40,113 events and lost 1,083 selected observations; every loss was `contended`, with no full/poisoned/exhausted loss. These are lost diagnostic observations, not evidence of dropped Raft messages. Deterministic context-suffix sampling, missing observations and the finite prefix prevent treating the remaining sample as an unbiased latency population. `stage-counts.csv` preserves all 23 rows for each of six processes, including zero counts and recorded message kinds.

## Ticket-local spans that remain available

Every matched duration below is marked partial observation. Ticket/key/route identity and local timestamp ordering are retained. Spans overlap and have differing populations; neither means nor follower durations may be added or subtracted to produce request latency. Accepted→dequeued can be reversed because admission is recorded after publication; reversed durations remain unavailable, never clamped to zero.

| Process | Span | Count | Sum ns | Mean µs | p50 µs | p95 µs | p99 µs | Unmatched | Reversed |
|---|---|---|---|---|---|---|---|---|---|
| r0/n1 | inbox_admitted_to_inbox_drained | 580 | 1079278 | 1.860824 | 1.616–1.631 | 3.040–3.071 | 4.416–4.479 | 0 | 0 |
| r0/n1 | inbox_offered_to_inbox_admitted | 580 | 124578 | 0.214790 | 0.100–0.100 | 0.368–0.371 | 2.656–2.687 | 0 | 0 |
| r0/n1 | inbox_offered_to_inbox_drained | 580 | 1203856 | 2.075614 | 1.728–1.743 | 4.160–4.223 | 4.736–4.799 | 0 | 0 |
| r0/n1 | outbound_accepted_to_outbound_dequeued | 567 | 916358 | 1.616152 | 1.456–1.471 | 2.304–2.335 | 3.136–3.167 | 10 | 3 |
| r0/n1 | outbound_offered_to_outbound_accepted | 580 | 384833 | 0.663505 | 0.500–0.503 | 1.488–1.503 | 3.200–3.231 | 0 | 0 |
| r0/n1 | outbound_offered_to_outbound_dequeued | 570 | 1286996 | 2.257888 | 1.984–1.999 | 3.968–3.999 | 5.504–5.567 | 10 | 0 |
| r0/n2 | inbox_admitted_to_inbox_drained | 996 | 1647306 | 1.653922 | 1.584–1.599 | 3.008–3.039 | 4.480–4.543 | 68 | 0 |
| r0/n2 | inbox_offered_to_inbox_admitted | 1034 | 205063 | 0.198320 | 0.100–0.100 | 0.460–0.463 | 2.464–2.495 | 30 | 0 |
| r0/n2 | inbox_offered_to_inbox_drained | 1026 | 1908154 | 1.859799 | 1.712–1.727 | 3.808–3.839 | 4.480–4.543 | 38 | 0 |
| r0/n2 | outbound_accepted_to_outbound_dequeued | 1029 | 1732298 | 1.683477 | 1.616–1.631 | 2.304–2.335 | 3.200–3.231 | 129 | 2 |
| r0/n2 | outbound_offered_to_outbound_accepted | 1160 | 543616 | 0.468634 | 0.468–0.471 | 2.080–2.111 | 2.944–2.975 | 0 | 0 |
| r0/n2 | outbound_offered_to_outbound_dequeued | 1031 | 2224011 | 2.157140 | 2.016–2.031 | 3.456–3.487 | 4.352–4.415 | 129 | 0 |
| r0/n3 | inbox_admitted_to_inbox_drained | 580 | 1086589 | 1.873429 | 1.616–1.631 | 3.872–3.903 | 4.800–4.863 | 0 | 0 |
| r0/n3 | inbox_offered_to_inbox_admitted | 580 | 133301 | 0.229829 | 0.110–0.110 | 0.448–0.451 | 2.656–2.687 | 0 | 0 |
| r0/n3 | inbox_offered_to_inbox_drained | 580 | 1219890 | 2.103259 | 1.744–1.759 | 4.224–4.287 | 5.632–5.695 | 0 | 0 |
| r0/n3 | outbound_accepted_to_outbound_dequeued | 569 | 977386 | 1.717726 | 1.552–1.567 | 2.336–2.367 | 3.776–3.807 | 10 | 1 |
| r0/n3 | outbound_offered_to_outbound_accepted | 580 | 402458 | 0.693893 | 0.508–0.511 | 1.920–1.935 | 3.136–3.167 | 0 | 0 |
| r0/n3 | outbound_offered_to_outbound_dequeued | 570 | 1368384 | 2.400674 | 2.080–2.111 | 4.160–4.223 | 5.248–5.311 | 10 | 0 |
| r1/n1 | inbox_admitted_to_inbox_drained | 575 | 1051903 | 1.829397 | 1.616–1.631 | 2.688–2.719 | 4.288–4.351 | 0 | 0 |
| r1/n1 | inbox_offered_to_inbox_admitted | 575 | 121507 | 0.211317 | 0.100–0.100 | 0.420–0.423 | 2.656–2.687 | 0 | 0 |
| r1/n1 | inbox_offered_to_inbox_drained | 575 | 1173410 | 2.040713 | 1.728–1.743 | 4.000–4.031 | 4.800–4.863 | 0 | 0 |
| r1/n1 | outbound_accepted_to_outbound_dequeued | 564 | 958368 | 1.699234 | 1.504–1.519 | 2.496–2.527 | 3.328–3.359 | 10 | 1 |
| r1/n1 | outbound_offered_to_outbound_accepted | 575 | 375765 | 0.653504 | 0.500–0.503 | 1.408–1.423 | 3.008–3.039 | 0 | 0 |
| r1/n1 | outbound_offered_to_outbound_dequeued | 565 | 1327510 | 2.349575 | 2.032–2.047 | 3.936–3.967 | 5.632–5.695 | 10 | 0 |
| r1/n2 | inbox_admitted_to_inbox_drained | 995 | 1596384 | 1.604406 | 1.568–1.583 | 2.304–2.335 | 4.000–4.031 | 59 | 0 |
| r1/n2 | inbox_offered_to_inbox_admitted | 1024 | 217842 | 0.212736 | 0.100–0.100 | 0.360–0.363 | 2.528–2.559 | 30 | 0 |
| r1/n2 | inbox_offered_to_inbox_drained | 1023 | 1868846 | 1.826829 | 1.680–1.695 | 3.584–3.615 | 4.352–4.415 | 31 | 0 |
| r1/n2 | outbound_accepted_to_outbound_dequeued | 1007 | 1714783 | 1.702863 | 1.648–1.663 | 2.240–2.271 | 3.488–3.519 | 142 | 1 |
| r1/n2 | outbound_offered_to_outbound_accepted | 1149 | 537392 | 0.467704 | 0.460–0.463 | 2.240–2.271 | 2.880–2.911 | 1 | 0 |
| r1/n2 | outbound_offered_to_outbound_dequeued | 1009 | 2197148 | 2.177550 | 2.032–2.047 | 3.808–3.839 | 4.288–4.351 | 141 | 0 |
| r1/n3 | inbox_admitted_to_inbox_drained | 575 | 1098906 | 1.911141 | 1.632–1.647 | 3.584–3.615 | 5.760–5.823 | 0 | 0 |
| r1/n3 | inbox_offered_to_inbox_admitted | 575 | 131014 | 0.227850 | 0.120–0.120 | 0.428–0.431 | 2.624–2.655 | 0 | 0 |
| r1/n3 | inbox_offered_to_inbox_drained | 575 | 1229920 | 2.138991 | 1.776–1.791 | 4.224–4.287 | 5.952–6.015 | 0 | 0 |
| r1/n3 | outbound_accepted_to_outbound_dequeued | 565 | 960305 | 1.699655 | 1.536–1.551 | 2.432–2.463 | 3.680–3.711 | 7 | 3 |
| r1/n3 | outbound_offered_to_outbound_accepted | 575 | 390327 | 0.678830 | 0.520–0.527 | 1.248–1.263 | 3.200–3.231 | 0 | 0 |
| r1/n3 | outbound_offered_to_outbound_dequeued | 568 | 1339330 | 2.357975 | 2.048–2.079 | 4.032–4.063 | 5.184–5.247 | 7 | 0 |

## Unavailable context timing

Global selected-observation loss disables every context/message timing join in each prefix, even where endpoint candidates appear locally plausible. The frozen reader remains unchanged. No cross-process clock subtraction, first-response matching or synthetic zero latency is used. Group terms remain unobserved under this loss rule. CompletionEligible is a selected registry member observation, not successful client delivery or proof of which follower closed quorum.

| Leader prefix | Context candidate span | Unavailable: loss | Unavailable: repeated context/term | Matched |
|---|---|---|---|---|
| trace-c1-r0 | leader_dequeued_to_response_validated | 1160 | 0 | 0 |
| trace-c1-r0 | leader_group_submit_to_heartbeat_outbound_offered | 1160 | 0 | 0 |
| trace-c1-r0 | leader_group_submit_to_heartbeat_outbound_accepted | 1160 | 0 | 0 |
| trace-c1-r0 | leader_response_validated_to_driver_step | 1160 | 0 | 0 |
| trace-c1-r0 | leader_response_driver_step_to_quorum_confirmed | 1160 | 0 | 0 |
| trace-c1-r1 | leader_dequeued_to_response_validated | 1146 | 2 | 0 |
| trace-c1-r1 | leader_group_submit_to_heartbeat_outbound_offered | 1146 | 2 | 0 |
| trace-c1-r1 | leader_group_submit_to_heartbeat_outbound_accepted | 1146 | 2 | 0 |
| trace-c1-r1 | leader_response_validated_to_driver_step | 1146 | 2 | 0 |
| trace-c1-r1 | leader_response_driver_step_to_quorum_confirmed | 1146 | 2 | 0 |

For the dequeue→response candidate alone, all 2,308 observed leader-edge candidates are unavailable: 2,306 classified as observation loss and two as repeated context/term ambiguity. These are local candidate-edge counts, not distinct requests. Every other unavailable group/follower/message span and reason is retained in `span-accounting.csv`; empty distributions retain null latency fields in `summary.json`. The recorded losses block the requested complete-context conclusion; they do not invalidate the successfully accounted client cohorts or the explicitly partial ticket observations.

## Source, acceptance and limitations

| Role | Revision | Server SHA-256 |
|---|---|---|
| control | 11113f68f6a5df77da1ffb4fcec850953716ffa3 | dd028cb2f61633dda133b05d814a6d173a79a83145d8ced2f81dc0cf8e0f33bc |
| trace | 1875e74141753cc6a55f025384b480b71ee84c1a | fe19660ed7182541c012f008a91eb532fc4e92f61be4103192dd53ae66b56762 |

Control uses empty production features. Trace uses root/server `quorum-trace`, Raft `quorum-trace` plus `read-stage-timing`, and an unchanged empty engine feature set. Both retain ThinLTO release codegen; fixed client revision is `0be806d9671e2c50701a64aa7889c8859b7648ba`, binary `8da9af469f962a938027d1970141bbe4622f7d42b2b795f720e288fb3f8d5957`. Runtime `15017 / 0 / c5772f` and independent readback `55391 / 0 / 468553` are distinct terminal executions. The audit accepted 16 exited lifetimes, 12 fresh drains, 24 endpoint metric documents, 12 voter/listener bindings and exact outer restoration. It independently verified 144 retained fixture files totaling 32,567,058 bytes.

The original helper preparation and its correction history remain intact. The initial 13-test campaign run passed 12 and failed one negative test because it expected ValueError while the preserved validator raised AssertionError (`057aaa/1`). Only that test exception expectation changed; the one repaired control passed `8e328d/0`, without rerunning the other 12. Earlier reader revisions and the complete-chain/identity corrections, all original controls and the read-only discovery failure are inventory-bound. They are preparation history, not repeated production measurements.

The bounded inventory references original source/report/trace bytes and the accepted full input inventory. It is not a WAL, executable or payload backup and does not claim standalone replay of the entire fixture audit. The separately archived [original evidence](original-evidence.tar.gz) and [archive inventory](archive-inventory.json) retain root's full selected acceptance inputs: archive SHA-256 `7eb6413a12a4bebdb62762506ca021773f0abf7a4fffc8130a0255662b04fb22`, 18,936,215 stored bytes / 98,045,608 raw bytes, independently read back by root (`83c66c/0`). This report did not reprocess that archive. The next collector implementation, any further capture and any acceptance of complete RTT require fresh qualification; none is inferred here.
