# @kv9/client (TypeScript / Node.js)

Promise-based client for kv9 on `@grpc/grpc-js`. Node 18+. The proto is loaded at runtime from
`crates/server/proto/kv9.proto` (override with `KV9_PROTO` or the `protoPath` option), so no code
generation step is needed.

```bash
cd sdk/typescript && npm install && npm run build
```

```ts
import { Client, Mutation } from "@kv9/client";

const client = new Client("127.0.0.1:20161", "my-client-token", {
  peers: { 1: "127.0.0.1:20161", 2: "127.0.0.1:20162", 3: "127.0.0.1:20163" }, // follows NotLeader hints
  timeoutMs: 5000,
});
const ks = client.keyspace(await client.createKeyspace("app"));   // epoch (1, 1)

const applied = await ks.put(Buffer.from("user/1"), Buffer.from("alice")); // { term, index }
await ks.batch([                                                            // ONE Raft entry, in order
  Mutation.put(Buffer.from("user/2"), Buffer.from("bob")),
  Mutation.del(Buffer.from("user/1")),
]);
await ks.batchGet([Buffer.from("user/1"), Buffer.from("user/2")]);        // [null, <Buffer bob>]
for await (const [key, value] of ks.scanAll(Buffer.from("user/"), Buffer.from("user0"))) { /* … */ }
await ks.deleteRange(Buffer.from("user/"), Buffer.from("user0"));
client.close();
```

Keys and values are `Buffer`s. A `Keyspace` starts at epoch `(1, 1)` and refreshes it once through
`GetRegion` when the server answers `FAILED_PRECONDITION` without a marker (pass `autoRefresh = false` to
`client.keyspace(...)` to disable; `refreshEpoch()` does it by hand).

## Errors

| Class | When | Safe to retry? |
| --- | --- | --- |
| `NotLeaderError` (`leaderNodeId`) | not the leader; nothing executed | yes (already followed when `peers` knows the node) |
| `ReadUnconfirmedError` (`phase`) | the read barrier did not establish | yes |
| `AdmissionRefusedError` (`reason`) | refused before execution | after backing off |
| `PartialWriteError` | `deleteRange` committed some chunks, then failed | continue from the reported position |
| `RpcError` (`code`, `details`, `.precondition`) | unmarked status | a write's outcome is unknown |
| `BoundsError` | local bound violated; nothing sent | fix the request |
| `ProtocolError` | unreadable marker | no |

Bounds: key ≤ 4096 B, value ≤ 65 536 B, 1..=256 items per batch, ≤ 1 MiB per message, ≤ 30 s per call.
`uint64` fields (terms, indexes, ids) are delivered as JavaScript numbers.

## Tests

```bash
npm test                 # marker classification (node:test)
scripts/sdk-e2e.sh       # boots a cluster and runs `npm run smoke` against it
```
