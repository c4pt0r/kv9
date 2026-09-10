# Native batch client-link fault preflight

The local preflight established effective fault selectors for the native
streaming client's ordinary Service connections. It also qualified a TCP reset
helper against the same running SDK process. This is preparation for native
batch fault acceptance, not completion of the full Chaos Mesh matrix or a
performance measurement.

## Actual effects

The fixture used three WAL voters, three ordinary ClusterIP Services, a
dedicated native client Pod and a separate control-client Pod. All resources
belonged to one uniquely labeled namespace. Simultaneous probes from the
control client and all three voters checked that faults stayed on the selected
client lane. The four link windows used the installed Chaos Mesh controller.

| Injection from the native Pod | Service connections | Direct voter Pod connections | Simultaneous control probes |
| --- | --- | --- | --- |
| Voter Pod targets, 250 ms delay | 9/9 connected; mean 1.29 ms | 9/9 connected; mean 251.74 ms | 72/72 connected |
| Exact Service VIP targets, 250 ms delay | 9/9 connected; mean 251.68 ms | 9/9 connected; mean 1.35 ms | 72/72 connected |
| Exact Service VIP targets, 100% loss | 0/9 connected within 750 ms | 9/9 connected | 72/72 connected |
| Exact Service VIP targets, partition | 0/9 connected within 750 ms | 9/9 connected | 72/72 connected |

These TCP probes include process launch overhead. Their purpose is to establish
the injected packet path. They do not measure database request latency.
Independent control PUT/GET pairs succeeded in every phase and all three
voters advanced their applied indexes. Netem recorded 36 dropped packets in
the total-loss window; the qualified reachable legacy-table DROP counter grew
from 3 to 39 in the partition window.

The cluster uses kube-proxy iptables Service DNAT. The native source network
namespace sees the Service VIP before node DNAT selects a voter Pod. Matching
only the voter Pod destination therefore missed ordinary Service traffic.
Actual netem, conntrack, DNAT, packet-counter and control observations establish
this distinction; the Chaos object's `AllInjected` condition alone does not.

After healing, all 90 final TCP probes connected and no DROP rule was reachable
from INPUT, OUTPUT or FORWARD. Chaos retained some unreferenced legacy chains;
their mere presence is not an active fault. The reachability observer was
checked against both the actual injected graph and a goto variant.

## Qualified connection reset

The final reset used `ss -4 -t -K state established` inside only the native
client's network namespace and matched one owned connection tuple. The client
remained PID 657, process start ticks 135897510 and boot ID
`455869d6-cdc2-4933-85cb-743fb8fcb02e`. Socket inode 165527798 disappeared and
replacement inode 165475219 appeared. TCP OutRsts, EstabResets and ActiveOpens
each increased by one; the command's stderr was empty.

This is a non-Chaos Linux SOCK_DESTROY helper. The installed NetworkChaos CRD
does not expose a connection-reset action. Its evidence must remain distinct
from the actual Chaos Mesh delay/loss/partition windows. The SDK process was
not restarted to manufacture connection recovery.

## Source and retained evidence

The source worktree remained clean at
`5cc98617b7e299e9f0fc3f46c65ef80ed4182c8a`. Executables came from the retained
default-feature build at `e246eae6ff48ef1f27dd8f0527b20ae48dc26280`; all 133
selected Rust, protobuf and Cargo runtime/build inputs matched the later
checkpoint. The build revision was not relabeled. A read-only owned hostPath
supplied these executables to the existing cached image:

- Server SHA-256: `f7e88b6c2f2514748c13fbaf85c3ee0540284138b3f3cc6bd774bdfd17e7eb4e`.
- Native client SHA-256: `6fb5633bd42cd34c9121d9bd8c04103b146beb79fcad52cb1fc8df41529db6d8`.

The unchanged native report validator accepted three complete histories:
1,765 calls (1,727 successful, 38 unknown), 181 calls (all successful), and
328 calls (314 successful, 14 unknown). All 2,274 calls and their terminal
outcomes remain; unknown writes were not replayed by the harness. These are
bounded preflight histories, separate from the forthcoming matrix's complete
fault-window and replica-drain obligations.

The retained root is `/tmp/kv9-native-batch-link-preflight`:

| Artifact | SHA-256 |
| --- | --- |
| Full `README.md` handoff | `47a5800d460d3b1744131b080b11b6309518f8f6f6998678cd2d1e997a37b260` |
| `independent-audit3/audit.json` | `03eecc3a7c98578112e93c29544157e4c722d151e8cdf7f133789367de1393af` |
| `cleanup/summary.json` | `a24e46434ae60b0c58065c73b3b86b63609376246b7bad64088c34162cd8e0e8` |
| `cleanup/owned-data.tar` | `a38a66a021ecc0943a81b091f6cfa2f9cbcaae18390f7cbe707f8f70a1b9e36c` |

The complete handoff inventory has SHA-256
`9daa18501d769b8d7ce4cab424bd54413d7cce6c95f8ef392b59fc0b00d573b9`
and covers 1,375 files / 59,630,640 bytes, all read back. The 14,090,240-byte
owned-data archive contains 61 files / 14,030,580 payload bytes compared against
the stable node originals before deletion. Local retained artifacts are not
embedded in Git; these hashes identify the audited copies.

Only namespace `kv9-native-link-178907-preflight`, UID
`38da05f7-eb76-4ad6-899b-68fc1b34aef6`, was created and removed. All five owned
Pod/container processes exited, owned faults disappeared, and the owned node
path was removed after verified capture. All eight historical namespace UIDs
remained unchanged. Host helpers ran on CPUs 6-31; initial Pod processes used
0-31 and later qualification pinned their threads to 6-31. Concurrent proof
work and a shared single-node Kind host preclude isolated timing claims.

## Requirements for subsequent acceptance

Before each client-link window, discover the current client's peer addresses
and bind them to Service UID/VIP/port and EndpointSlice Pod UID/IP. Use only the
owned namespace and native-client labels, `direction: to`, and those current
Service VIPs as `externalTargets`. On an address change, remove the old fault,
refresh the binding and qualify effects again. Never reuse preflight addresses.

The 100% loss run qualifies target selection and total-loss effects. A separate
partial-loss availability window remains required; the proposed 30% loss,
zero-correlation, 20-second window has not run. It must retain actual loss and
retransmission evidence, contained successful native calls, all unknown/refused
outcomes and unaffected control paths. Quorum loss/recovery and the complete
native batch Chaos matrix also remain open in #50.

For stream interruption, restrict the reset to the live SDK's exact owned
IPv4/TCP/ESTABLISHED tuple and bind replacement sockets to the unchanged
PID/start/boot/executable. Keep this helper explicitly labeled non-Chaos.

All preparation and observer failures remain in the handoff: archive lookup,
cached image digest resolution, script syntax, unavailable optional Perl
module, default-protocol `ss` diagnostics, an overly strict orphan-chain
healing check, helper import, and image-config-versus-manifest assumptions.
Corrections qualified the relevant observers without rewriting failed records
as successful. No production source, main-branch promotion or hosted CI
dispatch accompanies this preflight.
