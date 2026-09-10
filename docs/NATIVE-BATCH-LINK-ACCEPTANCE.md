# Native batch client-link and quorum-loss acceptance

Exact `5cc98617b7e299e9f0fc3f46c65ef80ed4182c8a` passed the dedicated local
11-window fixture and both independent readbacks on 2026-09-10. Actual attempt 4
was session **10968**, runner PID **1312319**, terminal exit **0**. All earlier
failed attempts remain preserved. This is one scoped link/quorum correctness
increment; it does **not** complete the [#50 umbrella](https://github.com/c4pt0r/kv9/issues/50).

The fixture is separate from the original 21-window voter/PVC matrix. It uses
one owned Kind node, client/voter CPUs 6–31, and data in an owned `/tmp` hostPath
on the node's **tmpfs**. It provides no cross-host, disk/power-loss durability,
throughput or liveness claim. Inter-voter partial loss, storage stalls and the
full adapter/ownership proof require separate acceptance.

## Exact source and execution

- Runtime revision: `5cc98617b7e299e9f0fc3f46c65ef80ed4182c8a`.
- Source-tree SHA256: `2412bc061227189a93c5d326e752d5991914a5a7438568d69abf43c7c4078ddd`.
- Standalone debug server SHA256: `64ec25c6454a3a8bb080a9b65cbea22945cd8e5a25e45ea65661f8b330123745`.
- Same-source native correctness client SHA256: `9bf536888a70e915c17c3b19016640b7c176b80bb429624d6625feb0cbb36e7d`.
- Image: `kv9-chaos:native-batch-5cc9861-20260910-attempt1`; actual CRI digest:
  `sha256:d08204d69f58342a123ef7cfc7291987454c47d34a53d280b1266d63aea23d9d`.

Both retained standalone Cargo graphs have empty root/engine/Raft/server feature
arrays. The source/build files were rechecked unchanged. Three executing voters
and the persistent client were bound to Pod UID, PID, Linux start ticks, boot ID
and executable hashes; 37 server samples and 24 client captures were replayed
from original command bytes. The same client process remained PID32, start ticks
136638508, boot `455869d6-cdc2-4933-85cb-743fb8fcb02e`.

Every window binds the current three Service UIDs/VIPs/20160-TCP ports through
EndpointSlice ownership to exact target Pod UIDs/IPs. The client used these
ordinary Service VIPs throughout. Its configuration remained four workers,
four shared keys, eight ordered batch items (including duplicates), 128-byte
values, mix 10/10/10/35/35, 500-ms intervals and 1500-ms logical deadlines.
No manual retry was introduced; retained SDK attempts passed the existing
unknown-write retry rule. Codec, stream-generation and response-ownership refinement remains outside the
operation-history proof scope.

## All contained windows

Each cell is **successful / unknown / refused completed batch calls** whose
invocation and return both lie inside the independently checked wall interval.
A batch call counts once, regardless of its item count. Calls spanning boundaries
remain in the complete history and are excluded from these contained counts.

| Window | Seconds | BatchGet S/U/R | BatchPut S/U/R |
|---|---:|---:|---:|
| baseline | 16.491 | 46 / 0 / 0 | 47 / 0 / 0 |
| vip-delay | 24.828 | 45 / 0 / 0 | 44 / 0 / 0 |
| delay-healed | 17.071 | 48 / 0 / 0 | 47 / 0 / 0 |
| vip-partial-loss | 35.399 | 64 / 5 / 0 | 61 / 8 / 0 |
| loss-healed | 17.709 | 50 / 0 / 0 | 48 / 0 / 0 |
| vip-partition | 18.120 | 0 / 15 / 0 | 0 / 14 / 0 |
| partition-healed | 18.283 | 51 / 0 / 0 | 52 / 0 / 0 |
| exact-tcp-reset | 18.663 | 52 / 0 / 0 | 52 / 0 / 0 |
| reset-healed | 19.035 | 53 / 0 / 0 | 54 / 0 / 0 |
| quorum-loss | 19.235 | 0 / 0 / 49 | 0 / 0 / 49 |
| quorum-healed | 19.523 | 54 / 0 / 0 | 55 / 0 / 0 |

Both unavailable windows require active completed batches, rather than client
silence. During full client partition, 15 BatchGet and 14 BatchPut calls completed
unknown; during quorum loss, 49 of each completed with typed pre-execution
refusals. **No wholly contained newly invoked point or batch operation succeeded
in either negative interval.** Every recovery interval contains successful native
BatchGet and BatchPut calls.

## Actual effects and unaffected controls

- **Service-VIP delay:** actual netem delay 250 ms; the nine selected native TCP
  probes took 251.349–251.857 ms. All 72 simultaneous non-native control/voter
  probes passed, maximum 1.741 ms. These timings establish an effect, not QPS.
- **Genuine partial loss:** exact 30% loss, correlation 0. The selected netem
  dropped-packet counter advanced 2→180, and native TCP RetransSegs 3→117.
  Successful batches and all observed unknowns were retained; no 100%-loss
  substitution or exact empirical 30% success-rate claim is made.
- **Full client partition:** nine native-to-VIP TCP probes failed; the remaining
  81 direct/control/voter probes passed. Reachable DROP packets advanced 5→82,
  and actual kernel IP sets included the bound VIPs. Separate CLI PUT/GET and
  all-voter apply progress continued on independent control keys.
- **Connection reset is explicitly non-Chaos:** exact-tuple Linux `SOCK_DESTROY`
  (`ss -4 -t -K`) removed the client's owned socket inode171004701, source
  port56340, to 10.96.184.187:20160. The same process acquired inode171045259,
  port55498. Raw FD listings, socket output and unchanged process identity were
  independently checked; the logical workload was not restarted.
- **Real quorum loss:** three directed NetworkChaos voter partitions covered
  both peer VIPs and Pod IPs. All 36 inter-voter TCP probes failed; 54 unaffected
  probes, including native/control access to voters, passed. Reachable DROP
  counters advanced 38→85, 46→92 and 23→65 on voters 1, 2 and 3. Each fault was
  observed injected/unrecovered at both interval boundaries.

The six actual NetworkChaos resources were deleted through their normal recovery
path. Healed windows had healthy probes and no active netem or reachable DROP.
Kernel set/rule/counter observations use the hash-bound private Debian ipset
observer; it was not installed globally or added to the runtime image.

## Complete history, drain and cleanup

The unchanged version-2 atomic history/report checker accepted all **2,146 calls**
and **4,292 events**: **1,839 successful, 86 unknown, 221 refused**. Traffic alone
contains 2,143 calls; initialization/verification add three successful calls.
The client ran 317.401 seconds and stopped by the explicit stop file, without
hitting a call/history/time cap. The independent witness check explored 5,547
states and accepted the complete retained history.

| Operation | Successful | Unknown | Refused |
|---|---:|---:|---:|
| batch_get | 647 | 29 | 77 |
| batch_put | 640 | 33 | 77 |
| delete | 184 | 7 | 23 |
| get | 186 | 6 | 23 |
| put | 182 | 11 | 21 |

The 86 unknown observations comprise **51 writes** (including 33 BatchPut calls)
and **35 reads**. Unknown reads are not classified as uncertain writes. Unknown
writes retain the checker's zero-or-whole atomic effect alternatives, including
permitted later effects; an unknown result is not treated as a definite failure.

After the client exited, four serial server captures established consecutive fresh
empty-public/read/apply Serving observations: qualifying advances **2/3/3** across
the three unchanged voters. Both status records in every qualifying sample were
empty, nonfatal and running. Their raw command intervals followed client exit,
and the same records are present in the full observation ledger.

The native PID exited; all five owned container lifetimes were verified gone.
Namespace `kv9-native-link-acceptance-20260910-4`, UID
`b00cc6e7-8f3a-4349-bf39-1cea0257a955`, is absent. Both the `/tmp` data directory
and independent `/opt` observer directory were removed after ownership and byte
checks. All eight preexisting namespace UIDs remain unchanged. Stable stores and
client files were archived before deletion; all 50 archive files were independently
read back against their retained originals.

## Preserved failures and evidence

1. Attempt 1: missing `docker exec -i` produced empty configuration/phase files;
   native initialization failed. Setup logs, empty bytes and cleanup are retained.
2. Attempt 2: native initialization succeeded, but the required ipset executable
   was absent. Baseline observation stopped before any fault; the native client
   drained during cleanup and its complete partial-run artifacts remain intact.
3. Attempt 3: Docker's archive-copy interface could not see the directory created
   by container exec. It stopped before namespace creation; owned staging cleanup
   passed.

The separate first copy prerequisite then caught `/tmp`'s `noexec` mount. Its
failure/source remains retained. The corrected prerequisite used stdin copy and
an independently owned executable `/opt` tool directory; actual tool execution,
hashes and cleanup passed before attempt 4. No mount flag, runtime binary,
validator or fault/drain predicate was weakened. All **63 synthetic rejection
controls**, the actual copy prerequisite, and every failed input are preserved.

Raw evidence: `/tmp/kv9-native-link-acceptance-attempt4`; its sibling `.log` and
`summary.json` retain terminal state. The frozen plan/helpers and prerequisite
records are under `/tmp/kv9-native-batch-link-acceptance-preparation-attempt4`.
Earlier attempts/preparations remain at their original numbered paths.
Independent records are under `/tmp/kv9-native-link-acceptance-independent`:

- `audit-first.json`: SHA256 `2caf9aec9efa18bfec87fe12ca5713757ae79fa0fab0209257d6279d636c322f`.
- `supplement.json`: SHA256 `8008273219311d7fb66db42d7fa0b3e18ff29fba65c3e800b93a5a7d3029d656`.
- `inventory.json`: SHA256 `f5676f6d8f1650e5d52de44789e268815043af1f129e28da0da5fc70aa25102f`.
- `inventory-readback.json`: SHA256 `ecc39dc549604b46bb8ff586a1589a4fa683a3c65cdcb7903c642e3fbce9ec83`.

The inventory binds **10,215 files / 488,506,506 bytes**
across raw attempts, preparations, exact source/build/tool inputs and audits.
Every file was independently reread unchanged, plus every regular member of all
three retained data archives. No originals were removed. No hosted CI was run;
the remaining umbrella obligations are unchanged.
