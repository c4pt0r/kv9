# Accepted gRPC socket configuration

Tracking: #41 and #9. This increment configures `TCP_NODELAY` on the sockets
accepted by the runtime's already-owned listener. The same listener serves
public requests, Raft streams and discovery. It is never released and rebound.

## Finding and change

The runtime previously supplied a plain `TcpListenerStream` to tonic's
`serve_with_incoming_shutdown`. In pinned tonic 0.14.6, server builder socket
options are ignored for caller-supplied incoming streams. Its client endpoints
enable `TCP_NODELAY` by default, but that does not configure the server's accepted
sockets. Consequently, setting `Server::tcp_nodelay(true)` alone would not repair
this path.

The runtime now wraps its existing listener in tonic's `TcpIncoming` and
explicitly enables `with_nodelay(Some(true))`. Tonic configures each accepted
socket before yielding it to the HTTP/2 server. This follows the pinned library's
normal listener configuration path. If the OS refuses the option, tonic logs a
warning and retains the connection; this option is not a consensus safety gate.

The b4a74b2 benchmark retains a 33.6–67.1 ms successful-read p99 interval at
concurrency 4 and above. That observation motivated inspection of TCP buffering;
it does not establish the cause of the tail. A separate exact-revision paired
run is required to attribute any effect to this socket change. The earlier
scheduler/notification benchmark must not be relabeled as this revision.

## Preserved obligations and local verification

This socket option changes when TCP offers small writes. It changes no message
encoding, ordering, destination generation, request receipt, quorum condition,
durability boundary, tick deadline or timeout budget. The existing proof treats
transport scheduling as asynchronous; this configuration grants no new consensus
authority and introduces no additional fairness or timing premise. It adds no
required database node, proxy or external service. The scheduling proof and its
implementation refinement remain separate open requirements.

The regression consumes the production incoming helper with a real loopback
listener. It proves the endpoint remains reserved and reads `TCP_NODELAY` back
from two actual accepted kernel sockets. It does not infer configuration from a
builder flag or use a narrow latency threshold. The server package passes 146
unit tests and six doctests, with one explicitly ignored test. Exact-candidate
process/Chaos acceptance and measured tail-latency improvement remain pending.
