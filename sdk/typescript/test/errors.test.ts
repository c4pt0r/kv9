import assert from "node:assert/strict";
import { test } from "node:test";
import { Metadata, status as Status } from "@grpc/grpc-js";

import {
  AdmissionRefusedError,
  NotLeaderError,
  PartialWriteError,
  ProtocolError,
  ReadUnconfirmedError,
  RpcError,
  classify,
} from "../src/errors.js";

function failed(code: Status, markers: Array<[string, string]> = [], details = "message") {
  const metadata = new Metadata();
  for (const [key, value] of markers) metadata.add(key, value);
  return { code, details, metadata };
}

test("markers decode to typed errors", () => {
  const leader = classify(
    failed(Status.FAILED_PRECONDITION, [
      ["kv9-not-leader", "true"],
      ["kv9-leader-node-id", "3"],
    ]),
    false,
  );
  assert.ok(leader instanceof NotLeaderError);
  assert.equal(leader.leaderNodeId, 3);
  const bare = classify(failed(Status.FAILED_PRECONDITION, [["kv9-not-leader", "true"]]), true);
  assert.ok(bare instanceof NotLeaderError && bare.leaderNodeId === null);

  const read = classify(failed(Status.UNAVAILABLE, [["kv9-read-unconfirmed", "quorum"]]), true);
  assert.ok(read instanceof ReadUnconfirmedError && read.phase === "quorum");

  const refused = classify(failed(Status.RESOURCE_EXHAUSTED, [["kv9-admission-refused", "encoded_bytes"]]), false);
  assert.ok(refused instanceof AdmissionRefusedError && refused.reason === "encoded_bytes");

  const partial = classify(
    failed(
      Status.UNAVAILABLE,
      [
        ["kv9-partial-write", "true"],
        ["kv9-committed-chunks", "2"],
        ["kv9-last-applied-term", "4"],
        ["kv9-last-applied-index", "77"],
      ],
      "leader changed",
    ),
    false,
  );
  assert.ok(partial instanceof PartialWriteError);
  assert.deepEqual(
    [partial.committedChunks, partial.lastAppliedTerm, partial.lastAppliedIndex, partial.cause],
    [2, 4, 77, "leader changed"],
  );
});

test("malformed or foreign markers are protocol errors", () => {
  const cases: Array<[Status, Array<[string, string]>, boolean]> = [
    [Status.FAILED_PRECONDITION, [["kv9-not-leader", "yes"]], false],
    [Status.UNAVAILABLE, [["kv9-not-leader", "true"]], false],
    [
      Status.FAILED_PRECONDITION,
      [
        ["kv9-not-leader", "true"],
        ["kv9-leader-node-id", "0"],
      ],
      false,
    ],
    [
      Status.FAILED_PRECONDITION,
      [
        ["kv9-not-leader", "true"],
        ["kv9-leader-node-id", "03"],
      ],
      false,
    ],
    [Status.UNAVAILABLE, [["kv9-read-unconfirmed", "quorum"]], false],
    [Status.UNAVAILABLE, [["kv9-read-unconfirmed", "later"]], true],
    [Status.RESOURCE_EXHAUSTED, [["kv9-admission-refused", "mood"]], false],
    [Status.INTERNAL, [["kv9-something-new", "1"]], false],
  ];
  for (const [code, markers, read] of cases) {
    assert.throws(() => classify(failed(code, markers), read), ProtocolError, JSON.stringify(markers));
  }
});

test("unmarked statuses stay plain rpc errors", () => {
  const stale = classify(failed(Status.FAILED_PRECONDITION, [], "stale epoch"), false);
  assert.ok(stale instanceof RpcError && stale.precondition);
  const deadline = classify(failed(Status.DEADLINE_EXCEEDED), false);
  assert.ok(deadline instanceof RpcError && !deadline.precondition);
});
