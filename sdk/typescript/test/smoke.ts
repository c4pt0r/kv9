// End-to-end smoke used by scripts/sdk-e2e.sh. Reads KV9_ADDR, KV9_TOKEN, KV9_KEYSPACE
// and optionally KV9_PEERS (`1=host:port,2=…`). Run with `npm run smoke`.
import assert from "node:assert/strict";

import { BoundsError, Client, MAX_KEY_BYTES, Mutation } from "../src/index.js";

function env(name: string): string {
  const value = process.env[name];
  if (!value) throw new Error(`${name} must be set`);
  return value;
}

const peers: Record<number, string> = {};
for (const item of (process.env.KV9_PEERS ?? "").split(",").filter(Boolean)) {
  const [node, address] = item.split("=");
  peers[Number(node)] = address;
}

const client = new Client(env("KV9_ADDR"), env("KV9_TOKEN"), { peers });
const p = (suffix: string) => Buffer.from(`ts-smoke/${suffix}`);
try {
  const info = await client.clusterInfo();
  assert.ok(info.nodeCount >= 1);
  const ks = client.keyspace(Number(env("KV9_KEYSPACE")));

  const first = await ks.put(p("k1"), Buffer.from("v1"));
  assert.ok(first.term > 0 && first.index > 0);
  assert.deepEqual(await ks.get(p("k1")), Buffer.from("v1"));
  assert.equal(await ks.get(p("absent")), null);

  const second = await ks.batch([
    Mutation.put(p("k2"), Buffer.from("v2")),
    Mutation.put(p("k3"), Buffer.from("v3")),
    Mutation.del(p("k1")),
  ]);
  assert.ok(second.term > first.term || second.index > first.index);
  assert.deepEqual(await ks.batchGet([p("k1"), p("k2"), p("k3"), p("k2")]), [
    null,
    Buffer.from("v2"),
    Buffer.from("v3"),
    Buffer.from("v2"),
  ]);
  const keys: Buffer[] = [];
  for await (const [key] of ks.scanAll(p(""), p("\x7f"))) keys.push(key);
  assert.deepEqual(keys, [p("k2"), p("k3")]);

  await ks.batchPut([
    [p("k4"), Buffer.from("v4")],
    [p("k5"), Buffer.from("v5")],
  ]);
  const page = await ks.scan(p(""), p("\x7f"), 2);
  assert.equal(page.pairs.length, 2);

  const deleted = await ks.deleteRange(p(""), p("\x7f"));
  assert.ok(deleted.committedChunks >= 1);
  const left: Buffer[] = [];
  for await (const [key] of ks.scanAll(p(""), p("\x7f"))) left.push(key);
  assert.deepEqual(left, []);

  await assert.rejects(ks.put(Buffer.alloc(MAX_KEY_BYTES + 1), Buffer.alloc(0)), BoundsError);
  await assert.rejects(ks.batch([]), BoundsError);
  console.log(`typescript sdk smoke: OK (leader ${client.currentAddress})`);
} finally {
  client.close();
}
