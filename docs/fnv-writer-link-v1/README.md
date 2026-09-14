# FNV writer: eleven-window client-link and quorum evidence

Exact candidate `12f44d35590ede5f89337fe731dd950162865154` passed all eleven windows and the independent history, same-leaf and source checks. Runtime session69956 ended d310bf/0; post session38527 ended a8923f/0. Original records remain unchanged.

The complete history has **2,126 calls: 1,835 OK, 63 unknown, 228 refused**. The original zero-unknown-effect search was inconclusive; the retained guided search produced a valid full-history witness. No outcome was dropped or relabeled.

| Fully contained negative window | OK | Unknown | Refused |
|---|---:|---:|---:|
| Full client-VIP partition | 0 | 37 | 0 |
| All-voter quorum partition | 0 | 0 | 136 |

These populations use invocation and return times strictly inside the independently checked windows. Calls crossing boundaries remain in the full history and are not counted as negative-window successes. Client partition includes13 BatchGet and13 BatchPut unknowns; quorum loss includes48 BatchGet and48 BatchPut refusals.

Actual client-link delay,30% loss and partition plus three directed voter partitions use Chaos Mesh. The same native netem leaf5:, parent1:4, advances0→203 dropped packets. Parent and child counters are not summed. Exact TCP reset is separately identified as Linux SOCK_DESTROY: the same client lifetime replaces socket inode271312194 with271312697 against the same leader VIP. The eleven-window baseline/heal/reset/effect/history predicates remain unchanged.

Three serial post-client observations satisfy the original two-fresh-empty-exports-per-voter drain requirement. All five owned container lifetimes exit; the owned namespace and exact node/observer paths are removed. Fresh read-only receipt e44e99/0 confirms all eight protected namespace UIDs and historical PodChaos UIDc3ededa7-0746-4864-bd95-4425ceb3d663 remain unchanged. That historical fault is preserved; no healed claim is made for it.

The original runner and independent auditor retain/read back50 regular data/history files totaling12,739,545 bytes. The original12,789,760-byte `owned-data.tar` remains local. This portable package contains complete native history/report/configuration, command/effect/clock/observer metadata, all five control results, original preparation failure and corrected adaptation, source/image/build bindings, actual terminals and independent checks. The local archive, WALs, root descriptors, executable bytes and observer binary archive are omitted with explicit original hash/size references. The publisher hashes the existing full tar once without re-decoding its members; it does not re-audit local data or rerun the fixture.

`ACCEPTANCE.json` contains the exact operation/window readout and original input pins. `inventory.json` binds every portable member's original path, size and SHA-256, top-level copies, all2 MiB archive parts and the whole compressed stream. `verify.py` independently streams all parts and every archive member, checking gzip/trailing data, safe paths, bounded sizes, missing/duplicate members and SHA-256. It does not require or inspect live source/runtime/WAL files. Replaying the original full-data auditor still requires the explicitly retained local payloads and paths.

Limits are64 MiB/member,512 MiB decoded payload,512 MiB compressed,10,000 members and256 parts. Publication uses a1 GiB additional-space reservation above the96 GiB floor, five-second observations and a1200-second deadline. No benchmark duration, runtime guard or retention coverage is changed.

Scope remains one-host volatile-WAL client-link/quorum correctness. This complements the separately accepted full21 campaign; it is not power-loss/cross-host/FSYNC-stall acceptance, a performance result or promotion. No original evidence is deleted and no Git/GitHub action is performed by this package.
