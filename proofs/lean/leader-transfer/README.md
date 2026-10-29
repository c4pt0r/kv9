# Leader-transfer model

Finite-trace projection of one leadership transfer (docs/LEADER-TRANSFER.md §2): the
term's leader, whether it holds a usable lease certificate, the transfer window between
the verb's decision and raft's outcome, the transferee's observed membership and
progress, and the one bad event — a lease read served after the decision. raft's
single leader per term and the lease model's certificate validation are premises.

| Model element | Implementation |
| --- | --- |
| `nominate` (voter, caught up, idle, not self; revokes the lease) | `RaftPeer::transfer_leader` → `NodeDriver::transfer_leader`, guarded in `AdminApi::transfer_leader` |
| `leaseRead` | the lease fast read path; its certificate check fails closed after revocation |
| `complete` | raft `MsgTimeoutNow`: the transferee campaigns and wins a new term |
| `expire` | raft abandons the transfer after one election timeout |
| `grant` | the lease controller, never during a transfer |
| `observe` | `peer_progress` re-read at every decision |

Eleven theorems: safety initialization/preservation/reachability; no lease read after
the decision; the window holds no lease; nomination requires a caught-up voter that is
not the leader; a transfer is not nominated twice; completion advances the term to the
transferee without a lease; the window ends by completion or expiry and nothing else;
both outcomes are reachable. Six semantic mutation controls and two policy controls.

Source-mapped abstract reasoning with explicit premises: not verified Rust extraction;
silent about who wins the election after `MsgTimeoutNow` and about placement.
