# kv9 C++ SDK

Synchronous C++17 client for kv9 on `grpc++`. The library generates its protocol sources from
`crates/server/proto/kv9.proto` at build time (`KV9_PROTO_DIR` overrides the location).

Dependencies: CMake ≥ 3.16, a C++17 compiler, protobuf and grpc++ development packages
(Ubuntu: `apt install libgrpc++-dev protobuf-compiler-grpc libprotobuf-dev`).

```bash
cmake -S sdk/cpp -B sdk/cpp/build -DCMAKE_BUILD_TYPE=Release
cmake --build sdk/cpp/build -j
(cd sdk/cpp/build && ctest)             # marker classification + local bounds
```

Link the `kv9_client` target (or add this directory with `add_subdirectory`) and include `kv9/client.h`:

```cpp
#include "kv9/client.h"

kv9::ClientOptions options;
options.peers = {{1, "127.0.0.1:20161"}, {2, "127.0.0.1:20162"}, {3, "127.0.0.1:20163"}};  // NotLeader hints
kv9::Client client("127.0.0.1:20161", "my-client-token", options);

auto id = client.CreateKeyspace("app");           // kv9::Expected<uint32_t>
if (!id) { /* id.error().kind / .message */ }
auto ks = client.Keyspace(*id);                   // epoch (1, 1), one-shot auto refresh

auto applied = ks.Put("user/1", "alice");         // Expected<Applied{term, index}>
auto value = ks.Get("user/1");                    // Expected<std::optional<std::string>>
ks.Batch({kv9::Mutation::Put("user/2", "bob"),    // ONE Raft entry, in order, all or nothing
          kv9::Mutation::Delete("user/1")});
auto rows = ks.ScanAll("user/", "user0");         // follows resume_from
ks.DeleteRange("user/", "user0");                 // chunked; see kPartialWrite
```

Every call returns `kv9::Expected<T>`: `ok()` / `operator bool`, `*result` for the value,
`result.error()` otherwise. `kv9::Error::kind` is one of:

| `ErrorKind` | When | Safe to retry? |
| --- | --- | --- |
| `kNotLeader` (`leader_node_id`) | not the leader; nothing executed | yes (already followed when `peers` knows the node) |
| `kReadUnconfirmed` (`phase`) | the read barrier did not establish | yes |
| `kAdmissionRefused` (`reason`) | refused before execution | after backing off |
| `kPartialWrite` (`committed_chunks`, `last_applied`) | `DeleteRange` committed some chunks, then failed | continue from `last_applied` |
| `kRpc` (`code`, `precondition()`) | unmarked status | a write's outcome is unknown |
| `kBounds` | local bound violated; nothing sent | fix the request |
| `kProtocol` | unreadable marker | no |

Bounds: key ≤ 4096 B, value ≤ 65 536 B, 1..=256 items per batch, ≤ 1 MiB per message, ≤ 30 s per call.
`Client` is copyable and thread-safe for concurrent calls; a `Keyspace` handle is not (its epoch is mutable
state), so give each thread its own.

`scripts/sdk-e2e.sh` builds `examples/smoke.cc` and runs it against a freshly booted cluster.
