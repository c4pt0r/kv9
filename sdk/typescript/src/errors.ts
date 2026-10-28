/**
 * Typed outcomes of the kv9 public API, decoded from the trailing-metadata markers the
 * server sends (see sdk/README.md). Anything the markers do not name stays a plain
 * {@link RpcError}; for a write that means the outcome is unknown.
 */
import { Metadata, status as Status } from "@grpc/grpc-js";

export const NOT_LEADER_KEY = "kv9-not-leader";
export const LEADER_HINT_KEY = "kv9-leader-node-id";
export const READ_UNCONFIRMED_KEY = "kv9-read-unconfirmed";
export const ADMISSION_REFUSED_KEY = "kv9-admission-refused";
export const PARTIAL_WRITE_KEY = "kv9-partial-write";
export const COMMITTED_CHUNKS_KEY = "kv9-committed-chunks";
export const LAST_APPLIED_TERM_KEY = "kv9-last-applied-term";
export const LAST_APPLIED_INDEX_KEY = "kv9-last-applied-index";

export class Kv9Error extends Error {
  constructor(message: string) {
    super(message);
    this.name = new.target.name;
  }
}

/** The request violates a documented bound; nothing was sent. */
export class BoundsError extends Kv9Error {}

/** The server answered with a marker the SDK cannot interpret. */
export class ProtocolError extends Kv9Error {}

/** An RPC failed without a typed marker. A write's outcome is unknown. */
export class RpcError extends Kv9Error {
  constructor(
    public readonly code: Status,
    public readonly details: string,
  ) {
    super(`${Status[code]}: ${details}`);
  }

  /** FAILED_PRECONDITION without a marker: stale epoch, cross-region batch or API-type mismatch. */
  get precondition(): boolean {
    return this.code === Status.FAILED_PRECONDITION;
  }
}

/** The addressed node is not the leader. Nothing executed; retry elsewhere. */
export class NotLeaderError extends Kv9Error {
  constructor(public readonly leaderNodeId: number | null) {
    super(`not leader (hint: node ${leaderNodeId})`);
  }
}

/** The read barrier did not establish within the deadline; nothing was answered. */
export class ReadUnconfirmedError extends Kv9Error {
  constructor(public readonly phase: "quorum" | "apply") {
    super(`read unconfirmed (${phase})`);
  }
}

/** Refused before execution; back off and retry. */
export class AdmissionRefusedError extends Kv9Error {
  constructor(public readonly reason: "request_count" | "encoded_bytes" | "request_too_large") {
    super(`admission refused (${reason})`);
  }
}

/** A delete_range committed some chunks and then failed. The committed chunks stay committed. */
export class PartialWriteError extends Kv9Error {
  constructor(
    public readonly committedChunks: number,
    public readonly lastAppliedTerm: number,
    public readonly lastAppliedIndex: number,
    public readonly cause: string,
  ) {
    super(
      `partial write: ${committedChunks} chunk(s) committed through (${lastAppliedTerm}, ${lastAppliedIndex}): ${cause}`,
    );
  }
}

/** The shape of a grpc-js ServiceError that classification needs. */
export interface StatusLike {
  code: Status;
  details?: string;
  metadata?: Metadata;
}

function values(metadata: Metadata | undefined, key: string): string[] {
  if (!metadata) return [];
  return metadata.get(key).map((v) => (typeof v === "string" ? v : v.toString("utf8")));
}

function single(metadata: Metadata | undefined, key: string): string | undefined {
  const found = values(metadata, key);
  if (found.length === 0) return undefined;
  if (found.length !== 1) throw new ProtocolError(`marker ${key} repeated`);
  return found[0];
}

function integer(metadata: Metadata | undefined, key: string): number {
  const text = single(metadata, key);
  if (text === undefined || !/^(0|[1-9][0-9]*)$/.test(text)) {
    throw new ProtocolError(`marker ${key} is not a canonical integer`);
  }
  return Number(text);
}

/** Turn a failed call into the typed error its markers describe. */
export function classify(error: StatusLike, read: boolean): Kv9Error {
  const { code, metadata } = error;
  const message = error.details ?? "";
  const keys = metadata ? Object.keys(metadata.getMap()) : [];
  const has = (key: string) => keys.includes(key);

  if (has(NOT_LEADER_KEY)) {
    if (single(metadata, NOT_LEADER_KEY) !== "true" || code !== Status.FAILED_PRECONDITION) {
      throw new ProtocolError("malformed not-leader marker");
    }
    let leader: number | null = null;
    if (has(LEADER_HINT_KEY)) {
      leader = integer(metadata, LEADER_HINT_KEY);
      if (leader === 0) throw new ProtocolError("leader hint names node 0");
    }
    return new NotLeaderError(leader);
  }
  if (has(ADMISSION_REFUSED_KEY)) {
    const reason = single(metadata, ADMISSION_REFUSED_KEY);
    if (reason !== "request_count" && reason !== "encoded_bytes" && reason !== "request_too_large") {
      throw new ProtocolError(`unknown admission refusal ${reason}`);
    }
    return new AdmissionRefusedError(reason);
  }
  if (has(PARTIAL_WRITE_KEY)) {
    if (single(metadata, PARTIAL_WRITE_KEY) !== "true") throw new ProtocolError("malformed partial-write marker");
    return new PartialWriteError(
      integer(metadata, COMMITTED_CHUNKS_KEY),
      integer(metadata, LAST_APPLIED_TERM_KEY),
      integer(metadata, LAST_APPLIED_INDEX_KEY),
      message,
    );
  }
  if (has(READ_UNCONFIRMED_KEY)) {
    const phase = single(metadata, READ_UNCONFIRMED_KEY);
    if (!read || code !== Status.UNAVAILABLE || (phase !== "quorum" && phase !== "apply")) {
      throw new ProtocolError("malformed read-unconfirmed marker");
    }
    return new ReadUnconfirmedError(phase);
  }
  if (keys.some((key) => key.startsWith("kv9-"))) {
    throw new ProtocolError(`unexpected control marker(s): ${keys.filter((k) => k.startsWith("kv9-")).join(",")}`);
  }
  return new RpcError(code, message);
}
