/**
 * kv9 client SDK for Node.js (TypeScript), on @grpc/grpc-js. The proto is loaded at
 * runtime from the repository (`crates/server/proto/kv9.proto`) or from KV9_PROTO.
 *
 * ```ts
 * import { Client, Mutation } from "@kv9/client";
 * const client = new Client("127.0.0.1:20161", "token", { peers: { 1: "127.0.0.1:20161" } });
 * const ks = client.keyspace(await client.createKeyspace("app"));
 * await ks.put(Buffer.from("k"), Buffer.from("v"));
 * await ks.batch([Mutation.put(Buffer.from("a"), Buffer.from("1")), Mutation.del(Buffer.from("k"))]);
 * ```
 *
 * The contract shared by every kv9 SDK is in sdk/README.md.
 */
import { fileURLToPath } from "node:url";
import path from "node:path";
import * as grpc from "@grpc/grpc-js";
import { loadSync } from "@grpc/proto-loader";

import {
  BoundsError,
  Kv9Error,
  NotLeaderError,
  ProtocolError,
  RpcError,
  classify,
  type StatusLike,
} from "./errors.js";

export * from "./errors.js";

export const MAX_KEY_BYTES = 4096;
export const MAX_VALUE_BYTES = 65_536;
export const MAX_BATCH_ITEMS = 256;
export const MAX_MESSAGE_BYTES = 1_048_576;
export const MAX_DEADLINE_MS = 30_000;

/** The region epoch a request believes in. Metadata-group keyspaces live at (1, 1). */
export interface Epoch {
  confVer: number;
  version: number;
}

/** The exact applied position of a write. Compare both fields, never the index alone. */
export interface Applied {
  term: number;
  index: number;
}

export interface RegionLocation {
  regionId: number;
  epoch: Epoch;
  leaderNodeId: number | null;
}

export interface KeyspaceInfo {
  id: number;
  name: string;
  tenantId: number;
}

export interface ClusterInfo {
  nodeCount: number;
  keyspaceCount: number;
  regionCount: number;
}

export interface ScanPage {
  pairs: Array<[Buffer, Buffer]>;
  /** Nonempty when the walk paused at a foreign-leader chunk: continue exactly here. */
  resumeFrom: Buffer;
}

export interface DeleteRangeResult {
  committedChunks: number;
  lastApplied: Applied;
  resumeFrom: Buffer;
}

/** One entry of an atomic range batch; `value === null` means delete. */
export interface Mutation {
  key: Buffer;
  value: Buffer | null;
}

export const Mutation = {
  put(key: Buffer, value: Buffer): Mutation {
    return { key, value };
  },
  del(key: Buffer): Mutation {
    return { key, value: null };
  },
};

export interface ClientOptions {
  /** Per-call deadline in milliseconds (at most 30 000). Default 5 000. */
  timeoutMs?: number;
  /** node_id -> address, used to follow NotLeader hints. */
  peers?: Record<number, string>;
  /** How many hints to follow for one logical call. Default 3. */
  maxRedirects?: number;
  /** Override the proto location (default: this repository's crates/server/proto/kv9.proto). */
  protoPath?: string;
}

function defaultProtoPath(): string {
  if (process.env.KV9_PROTO) return process.env.KV9_PROTO;
  const here = path.dirname(fileURLToPath(import.meta.url));
  // src/ or dist/ -> sdk/typescript -> repository root
  return path.resolve(here, "..", "..", "..", "crates", "server", "proto", "kv9.proto");
}

// eslint-disable-next-line @typescript-eslint/no-explicit-any
type Stub = any;

const serviceCache = new Map<string, Stub>();

function serviceConstructor(protoPath: string): Stub {
  let ctor = serviceCache.get(protoPath);
  if (!ctor) {
    const definition = loadSync(protoPath, {
      keepCase: true,
      longs: Number,
      enums: Number,
      bytes: Buffer,
      defaults: true,
      oneofs: true,
    });
    const loaded = grpc.loadPackageDefinition(definition) as Stub;
    ctor = loaded.kv9.v1.Kv9;
    serviceCache.set(protoPath, ctor);
  }
  return ctor;
}

function checkKey(key: Buffer, what = "key"): void {
  if (!Buffer.isBuffer(key)) throw new BoundsError(`${what} must be a Buffer`);
  if (key.length > MAX_KEY_BYTES) throw new BoundsError(`${what} exceeds ${MAX_KEY_BYTES} bytes`);
}

function checkValue(value: Buffer): void {
  if (!Buffer.isBuffer(value)) throw new BoundsError("value must be a Buffer");
  if (value.length > MAX_VALUE_BYTES) throw new BoundsError(`value exceeds ${MAX_VALUE_BYTES} bytes`);
}

function checkCount(count: number): void {
  if (count < 1 || count > MAX_BATCH_ITEMS) {
    throw new BoundsError(`a batch carries 1..=${MAX_BATCH_ITEMS} items, got ${count}`);
  }
}

function applied(r: { applied_term: number; applied_index: number }): Applied {
  if (!r.applied_term || !r.applied_index) {
    throw new ProtocolError("write acknowledged without an applied position");
  }
  return { term: r.applied_term, index: r.applied_index };
}

/** An authenticated client for one kv9 cluster. */
export class Client {
  private address: string;
  private readonly metadata = new grpc.Metadata();
  private readonly timeoutMs: number;
  private readonly peers: Record<number, string>;
  private readonly maxRedirects: number;
  private readonly protoPath: string;
  private readonly stubs = new Map<string, Stub>();

  constructor(address: string, token: string, options: ClientOptions = {}) {
    if (!token) throw new BoundsError("client token must be non-empty");
    this.timeoutMs = options.timeoutMs ?? 5_000;
    if (!(this.timeoutMs > 0 && this.timeoutMs <= MAX_DEADLINE_MS)) {
      throw new BoundsError(`timeoutMs must be in (0, ${MAX_DEADLINE_MS}]`);
    }
    this.address = address;
    this.metadata.set("authorization", `Bearer ${token}`);
    this.peers = options.peers ?? {};
    this.maxRedirects = options.maxRedirects ?? 3;
    this.protoPath = options.protoPath ?? defaultProtoPath();
  }

  /** The node currently addressed (moves with followed hints). */
  get currentAddress(): string {
    return this.address;
  }

  close(): void {
    for (const stub of this.stubs.values()) stub.close();
    this.stubs.clear();
  }

  private stub(address: string): Stub {
    let stub = this.stubs.get(address);
    if (!stub) {
      const Kv9 = serviceConstructor(this.protoPath);
      stub = new Kv9(address, grpc.credentials.createInsecure(), {
        "grpc.max_receive_message_length": MAX_MESSAGE_BYTES,
        "grpc.max_send_message_length": MAX_MESSAGE_BYTES,
      });
      this.stubs.set(address, stub);
    }
    return stub;
  }

  /** @internal */
  async invoke<T>(method: string, request: unknown, read: boolean): Promise<T> {
    let hops = 0;
    for (;;) {
      const stub = this.stub(this.address);
      try {
        return await new Promise<T>((resolve, reject) => {
          stub[method](
            request,
            this.metadata,
            { deadline: Date.now() + this.timeoutMs },
            (err: StatusLike | null, response: T) => (err ? reject(err) : resolve(response)),
          );
        });
      } catch (raw) {
        const error = raw instanceof Kv9Error ? raw : classify(raw as StatusLike, read);
        if (error instanceof NotLeaderError && hops < this.maxRedirects && error.leaderNodeId !== null) {
          const target = this.peers[error.leaderNodeId];
          if (target && target !== this.address) {
            this.address = target;
            hops += 1;
            continue;
          }
        }
        throw error;
      }
    }
  }

  // ------------------------------------------------------------------ admin

  /** Create a raw keyspace served by the metadata group; returns its id. */
  async createKeyspace(name: string, tenantId = 0): Promise<number> {
    const r = await this.invoke<{ keyspace_id: number }>(
      "CreateKeyspace",
      { name, tenant_id: tenantId, api_type: 2 /* API_TYPE_RAW */ },
      false,
    );
    return r.keyspace_id;
  }

  async listKeyspaces(): Promise<KeyspaceInfo[]> {
    const r = await this.invoke<{ keyspaces: Array<{ id: number; name: string; tenant_id: number }> }>(
      "ListKeyspaces",
      {},
      true,
    );
    return r.keyspaces.map((k) => ({ id: k.id, name: k.name, tenantId: k.tenant_id }));
  }

  async clusterInfo(): Promise<ClusterInfo> {
    const r = await this.invoke<{ node_count: number; keyspace_count: number; region_count: number }>(
      "ClusterInfo",
      {},
      true,
    );
    return { nodeCount: r.node_count, keyspaceCount: r.keyspace_count, regionCount: r.region_count };
  }

  async getRegion(keyspaceId: number, key: Buffer = Buffer.alloc(0)): Promise<RegionLocation> {
    checkKey(key);
    const r = await this.invoke<{
      region: { region_id: number; epoch: { conf_ver: number; version: number }; leader_node_id?: number };
    }>("GetRegion", { keyspace_id: keyspaceId, key }, true);
    const leader = r.region.leader_node_id;
    return {
      regionId: r.region.region_id,
      epoch: { confVer: r.region.epoch.conf_ver, version: r.region.epoch.version },
      leaderNodeId: leader ? leader : null,
    };
  }

  keyspace(keyspaceId: number, epoch: Epoch = { confVer: 1, version: 1 }, autoRefresh = true): Keyspace {
    return new Keyspace(this, keyspaceId, epoch, autoRefresh);
  }

  /** The keyspace called `name`: one ListKeyspaces lookup, then `keyspace(id)`. Names are
   *  what people use; ids are what the wire carries. Rejects with RpcError(NOT_FOUND). */
  async keyspaceNamed(name: string, epoch: Epoch = { confVer: 1, version: 1 }, autoRefresh = true): Promise<Keyspace> {
    const found = (await this.listKeyspaces()).find((k) => k.name === name);
    if (!found) throw new RpcError(grpc.status.NOT_FOUND, `no keyspace named ${JSON.stringify(name)}`);
    const ks = new Keyspace(this, found.id, epoch, autoRefresh);
    ks.name = found.name;
    return ks;
  }
}

/** Raw operations on one keyspace under one region epoch. */
export class Keyspace {
  epoch: Epoch;
  /** Set when the handle was opened by name. */
  name: string | null = null;

  constructor(
    private readonly client: Client,
    public readonly id: number,
    epoch: Epoch,
    private readonly autoRefresh: boolean,
  ) {
    if (!(id > 0 && id <= 0xff_ffff)) throw new BoundsError("keyspace id must be a nonzero 3-byte value");
    this.epoch = epoch;
  }

  private context() {
    return { keyspace_id: this.id, region_epoch: { conf_ver: this.epoch.confVer, version: this.epoch.version } };
  }

  /** Re-read the region epoch from the cluster and adopt it. */
  async refreshEpoch(key: Buffer = Buffer.alloc(0)): Promise<Epoch> {
    this.epoch = (await this.client.getRegion(this.id, key)).epoch;
    return this.epoch;
  }

  private async call<T>(method: string, build: () => unknown, read: boolean, anchor: Buffer): Promise<T> {
    try {
      return await this.client.invoke<T>(method, build(), read);
    } catch (error) {
      if (!(error instanceof RpcError && error.precondition && this.autoRefresh)) throw error;
      const before = this.epoch;
      await this.refreshEpoch(anchor);
      if (before.confVer === this.epoch.confVer && before.version === this.epoch.version) throw error;
      return await this.client.invoke<T>(method, build(), read);
    }
  }

  // ------------------------------------------------------------------ reads

  async get(key: Buffer): Promise<Buffer | null> {
    checkKey(key);
    const r = await this.call<{ value: { found: boolean; value: Buffer } }>(
      "RawGet",
      () => ({ context: this.context(), key }),
      true,
      key,
    );
    return r.value?.found ? r.value.value : null;
  }

  /** One quorum-confirmed view; results align with `keys`, duplicates included. */
  async batchGet(keys: Buffer[]): Promise<Array<Buffer | null>> {
    checkCount(keys.length);
    keys.forEach((k) => checkKey(k));
    const r = await this.call<{ values: Array<{ found: boolean; value: Buffer }> }>(
      "RawBatchGet",
      () => ({ context: this.context(), keys }),
      true,
      keys[0],
    );
    if (r.values.length !== keys.length) throw new ProtocolError("batchGet returned a different cardinality");
    return r.values.map((v) => (v.found ? v.value : null));
  }

  /** One page of [start, end); an empty `end` means "to the keyspace end". */
  async scan(start: Buffer = Buffer.alloc(0), end: Buffer = Buffer.alloc(0), limit = 1024): Promise<ScanPage> {
    checkKey(start, "start");
    checkKey(end, "end");
    if (limit <= 0) throw new BoundsError("limit must be positive");
    const r = await this.call<{ pairs: Array<{ key: Buffer; value: Buffer }>; resume_from: Buffer }>(
      "RawScan",
      () => ({ context: this.context(), start, end, limit }),
      true,
      start,
    );
    return { pairs: r.pairs.map((p) => [p.key, p.value]), resumeFrom: r.resume_from ?? Buffer.alloc(0) };
  }

  /** Walk [start, end) page by page, following `resumeFrom`. */
  async *scanAll(start: Buffer = Buffer.alloc(0), end: Buffer = Buffer.alloc(0), page = 1024) {
    let cursor = start;
    for (;;) {
      const result = await this.scan(cursor, end, page);
      yield* result.pairs;
      if (result.resumeFrom.length > 0) cursor = result.resumeFrom;
      else if (result.pairs.length < page) return;
      else cursor = Buffer.concat([result.pairs[result.pairs.length - 1][0], Buffer.from([0])]);
    }
  }

  // ----------------------------------------------------------------- writes

  async put(key: Buffer, value: Buffer): Promise<Applied> {
    checkKey(key);
    checkValue(value);
    return applied(await this.call("RawPut", () => ({ context: this.context(), key, value }), false, key));
  }

  async delete(key: Buffer): Promise<Applied> {
    checkKey(key);
    return applied(await this.call("RawDelete", () => ({ context: this.context(), key }), false, key));
  }

  /** Put-only atomic batch: every pair lands at one applied position. */
  async batchPut(pairs: Array<[Buffer, Buffer]>): Promise<Applied> {
    checkCount(pairs.length);
    for (const [k, v] of pairs) {
      checkKey(k);
      checkValue(v);
    }
    return applied(
      await this.call(
        "RawBatchPut",
        () => ({ context: this.context(), pairs: pairs.map(([key, value]) => ({ key, value })) }),
        false,
        pairs[0][0],
      ),
    );
  }

  /**
   * The atomic range batch: ordered puts and deletes applied as ONE Raft entry. Every key
   * must lie in the same region; a batch crossing a region is refused whole
   * (FAILED_PRECONDITION), nothing is proposed, and the SDK never splits it. A later
   * mutation for the same key wins; deleting an absent key is a no-op.
   */
  async batch(mutations: Mutation[]): Promise<Applied> {
    checkCount(mutations.length);
    const wire = mutations.map((m) => {
      checkKey(m.key);
      if (m.value === null) return { delete: m.key };
      checkValue(m.value);
      return { put: { key: m.key, value: m.value } };
    });
    return applied(
      await this.call("RawBatch", () => ({ context: this.context(), mutations: wire }), false, mutations[0].key),
    );
  }

  /** Delete [start, end) in bounded chunks. Not atomic as a whole: see PartialWriteError and resumeFrom. */
  async deleteRange(start: Buffer, end: Buffer): Promise<DeleteRangeResult> {
    checkKey(start, "start");
    checkKey(end, "end");
    const r = await this.call<{
      committed_chunks: number;
      last_applied_term: number;
      last_applied_index: number;
      resume_from: Buffer;
    }>("RawDeleteRange", () => ({ context: this.context(), start, end }), false, start);
    return {
      committedChunks: r.committed_chunks,
      lastApplied: { term: r.last_applied_term, index: r.last_applied_index },
      resumeFrom: r.resume_from ?? Buffer.alloc(0),
    };
  }
}
