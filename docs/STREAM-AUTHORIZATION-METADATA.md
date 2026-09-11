# Immutable stream authorization metadata experiment

This isolated experiment starts from `f2c4e85` with the system allocator. It
removes repeated authorization HeaderMap construction on byte-identical frames
of an admitted stream. It does not include typed-dispatch or allocator changes.
Throughput and latency gains require a matched measurement; none are claimed
by this source argument.

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

Focused local checks passed: 37 point-related tests, 31 gRPC tests and 10 optional RPC tests (filters overlap), plus all-target experimental Clippy with warnings denied. The initial new role-change test expected the wrong status; it was corrected to the unchanged handler's PermissionDenied result before these checks. Production behavior was not changed to satisfy that expectation. Logs: `/tmp/kv9-stream-auth-metadata-local-first`. These checks do not establish performance or full fault acceptance.
