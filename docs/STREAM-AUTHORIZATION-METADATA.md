# Immutable stream authorization metadata experiment

This integration starts from `d084e5831528297508a5f1aa86258fbc1838266f`,
whose runtime is the selected CRC `ca0002c7`, including jemalloc and the selected
read-worker configuration. It reapplies the bounded adapter from
`94d8b9fe1b59c6b441267de7dc4352207dd7079c`, removing repeated authorization
HeaderMap construction on byte-identical frames of an admitted stream.
It retains the selected Raft and storage implementation. The vector-receipt
experiment is not included.

The earlier system-allocator experiment had a favorable GET screen and actual
Chaos Mesh evidence. Neither result transfers to this combination. Fresh
source checks, recovery, c1/c64 pure/mixed timing and exact-source Chaos
acceptance are required. Throughput and latency gains are not claimed here.

## Correspondence with the existing adapter

Opening authentication still receives the original complete metadata, before
the unchanged stream-slot acquisition. Only then does the stream handler retain
one immutable singleton map, reconstructed with the same string parse/insert
operations as the original frame path. It retains at most 4103 authorization
bytes. Missing, non-ASCII or oversized opening values disable reuse without
introducing an opening refusal. Extra, duplicate and binary opening headers,
and the opening authorization's sensitive flag, are not copied.

Frame shape and deadline checks remain in the original stream owner. In the
bounded handler task, authorization and payload limits precede authentication;
the cache is eligible only for exact encoded-byte equality. Every frame invokes
the Authenticator again, including those with malformed protobuf. Fresh
principals, roles, revocations, errors and authenticator side effects remain
effective. Different credentials use the original parse/insert path and never
replace or enlarge the singleton. No authentication result is cached.

After authentication, protobuf decoding, batch bounds, public client-role
validation, decoded-size admission and RequestContext construction retain their
original order. On a cache hit the internal tonic Request contains the decoded
message and freshly authenticated extension, with empty metadata. This internal
representation is safe only for the five directly called Raw handlers: their
unchanged implementations read identity from extensions and the protobuf body,
not request metadata. No interceptor or other handler receives that request.
Fallback frames, unary calls and the tarpc control retain their prior metadata
behavior. Any future metadata-dependent Raw handler must revisit this boundary.

The correspondence projects onto metadata entries, bytes and flags supplied to
authentication, returned AuthContext/status, admission, backend calls and wire
outcomes. Map addresses, incidental allocations, reference counts and header
drop times differ. This is not equivalence for arbitrary custom authenticators
that inspect object identity or allocation behavior, nor a machine-checked
proof of the whole adapter. Core Raft, read barriers, write acknowledgement,
storage and replay transitions are unchanged.

## Lifetime and bounds

The map is owned by the stream's Handler and its existing bounded handler jobs.
Each job still owns the stream permit and response reservation through
cooperative cancellation. Consequently maps are bounded by admitted stream
slots, and references by the existing per-stream job limit. There is no global
cache, map mutex, unbounded token accumulation or header ownership in detached
write completion. The service-level handler has no cached map.

## Verification and screening

Focused tests compare singleton contents/flags against the original frame path,
exercise duplicate/sensitive/extra/binary opening metadata, mismatched and
invalid credentials, invalid protobuf, size limits, optional cache eligibility
and exact authentication call counts. A real HTTP/2 stream changes principal
and role and then revokes the same credential; opening-only headers must never
appear in frame authentication. Existing five-operation parity, cancellation,
stream-slot and uncertain-write tests apply unchanged.

Performance screening follows focused correctness, an actual process-history
test and a smoke run. Keep the fixed native/Redis clients and matched resource
budget. Only a useful throughput and latency result warrants broader acceptance
and an exact-source Chaos campaign. No production promotion follows from the
implementation alone.

The current integration retains the original three adapter tests and real
stream principal/role/revocation test. Its five Raw handlers, authentication
extension reader and admission path must be checked against the original
adapter's metadata-independent boundary. Validation receipts are published
separately and must identify this source; prior test counts are not reused.
