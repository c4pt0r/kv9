#!/usr/bin/env python3
"""Actual leader container-kill with replicated owners for real checkpoint SSTs.

Extends the bounded checkpoint-recovery fixture. It registers an explicit test
owner closure; it does not certify production backfill or deletion eligibility.
"""
import hashlib
import importlib.util
import json
from pathlib import Path
import secrets
import struct
import time

spec = importlib.util.spec_from_file_location(
    "checkpoint_cell", Path(__file__).with_name("checkpoint-publication-chaos.py"))
base = importlib.util.module_from_spec(spec)
spec.loader.exec_module(base)
require = base.require


def sealed(data):
    return data + hashlib.sha256(data).digest()


class RetentionCell(base.Cell):
    def __init__(self, args):
        super().__init__(args)
        self.ledger_history = []

    def request(self, tag, payload=b""):
        return sealed(b"KV9RTX01" + self.root_digest + bytes([tag]) + payload)

    def token(self, name):
        return self.owners[name]["id"] + struct.pack(">Q", 1)

    def owner(self, name, phase):
        owner = self.owners[name]
        body = (b"KV9OWN01" + self.root_digest + owner["id"] + bytes([5])
                + struct.pack(">QQQ", 1, 1, 1) + owner["operation"] + self.subject
                + bytes([0, 0]) + struct.pack(">QB", 1, phase)
                + struct.pack(">I", len(self.resources)) + b"".join(self.resources))
        return sealed(body)

    def ledger_rpc(self, verb, *args, frame=None):
        request_pin = None
        if frame is not None:
            require(len(frame) <= 16*1024, "bounded retention request frame")
            digest = hashlib.sha256(frame).hexdigest()
            path = self.out / "ledger-requests" / (digest + ".bin")
            path.parent.mkdir(exist_ok=True)
            if not path.exists():
                path.write_bytes(frame)
            request_pin = base.pin(path)
            pod_path = "/tmp/kv9-retention-" + digest + ".bin"
            # An explicit NotLeader retry may dial another voter; every exact
            # frame is present there before the first attempt. No unknown retry.
            for n in (1, 2, 3):
                self.k("exec", "-i", "-n", self.namespace, self.pods[n], "--",
                       "/bin/sh", "-c", 'cat > "$1"', "request", pod_path, stdin=frame)
            args = (*args, "--request-file", pod_path)
        for attempt in range(8):
            n = self.leader_id
            start = time.time_ns()
            out, code = self.exec(self.pods[n], "/usr/local/bin/kv9", "client", verb,
                                  "--addr", self.services[n]+":20160",
                                  "--root-digest", self.root_digest.hex(), *args, allowed=(0, 1))
            stderr = (self.out / "commands" / f"{self.count:05d}" / "stderr").read_text()
            self.ledger_history.append({"verb": verb, "phase": self.phase, "args": list(args),
                "attempt": attempt, "node": n, "started_ns": start, "ended_ns": time.time_ns(),
                "request": request_pin, "exit_code": code, "output": out.decode(),
                "command_number": self.count})
            if code == 0:
                return dict(line.split("=", 1) for line in out.decode().splitlines())
            require("not_leader=true" in stderr + out.decode(), "retention outcome unknown; no retry")
            self.wait("retention leader after explicit refusal", self.agreement)
        raise RuntimeError("retention typed leader retry bound")

    def mutate(self, tag, payload=b"", changed=True):
        result = self.ledger_rpc("retention-apply", frame=self.request(tag, payload))
        kind = "mutation" if changed else "confirmation"
        require(result["retention_outcome"] == ("changed" if changed else "confirmed"),
                "retention mutation/confirmation distinction")
        require(int(result[kind+"_term"]) > 0 and int(result[kind+"_index"]) > 0,
                "retention exact applied position")
        return result

    def observe(self, name, phase):
        result = self.ledger_rpc("retention-owner", "--owner-id", self.owners[name]["id"].hex())
        require(result == {"found": "true", "owner_hex": self.owner(name, phase).hex()},
                "exact canonical owner/phase/generation/whole-resource readback: " + name)

    def through(self, index):
        rows = self.statuses()
        return (rows and all(int(r.get("driver_applied_index", "0")) >= index
                            for r in rows.values()) and self.agreement(rows))

    def before_fault(self):
        self.phase = "ledger-acquisition-before-leader-loss"
        self.root_digest = hashlib.sha256((self.out / "root.bin").read_bytes()).digest()
        selected = self.layout(self.leader_id, "ledger-subject").checkpoint
        require(selected and 0 < len(selected["files"]) <= 64, "real bounded checkpoint closure")
        self.subject = hashlib.sha256(json.dumps(selected, sort_keys=True, separators=(",", ":")).encode()).digest()
        self.resources = sorted(bytes([1]) + hashlib.sha256(f["key"].encode()).digest()[:16]
                                + bytes.fromhex(f["sha256"]) for f in selected["files"])
        require(len({r[:17] for r in self.resources}) == len(self.resources), "distinct fixture resource instances")
        self.owners = {name: {"id": secrets.token_bytes(16), "operation": secrets.token_bytes(32)}
                       for name in ("source", "destination")}
        self.save("ledger-subject.json", {"root_digest": self.root_digest.hex(),
            "subject_digest": self.subject.hex(), "subject_is_anchor": False,
            "subject": selected, "resources": [r.hex() for r in self.resources],
            "owner_ids": {name: o["id"].hex() for name, o in self.owners.items()},
            "resource_instance_scope": "Fixture identity derived from the exact existing object key; no deletion or instance-retirement authority."})
        (self.out / "sources" / Path(__file__).name).write_bytes(Path(__file__).read_bytes())
        self.save("ledger-source-binding.json", {"helper": base.pin(__file__),
            "base_helper": base.pin(Path(__file__).with_name("checkpoint-publication-chaos.py"))})
        self.mutate(0)
        self.mutate(1, struct.pack(">I", len(self.resources)) + b"".join(self.resources))
        acquired = self.mutate(2, self.owner("source", 1))
        confirmation = self.mutate(2, self.owner("source", 1), changed=False)
        require(confirmation["revision"] == acquired["revision"]
                and int(confirmation["confirmation_index"]) > int(acquired["mutation_index"]),
                "duplicate request has a fresh confirmation, not a recovered receipt")
        self.mutate(3, self.token("source"))
        share = self.mutate(4, self.token("source") + self.owner("destination", 1))
        self.observe("source", 2)
        self.observe("destination", 1)
        index = int(share["mutation_index"])
        self.wait("replicated ownership before leader loss", lambda: self.through(index))
        self.wait("checkpoint includes both overlapping owners", lambda: all(
            (self.layout(n).checkpoint or {}).get("index", 0) >= index for n in (1, 2, 3)))
        self.share = share
        self.save("ledger-before-status.json", self.statuses())

    def choose_victim(self):
        # The existing fixture normally kills a follower. This cell specifically
        # kills the agreed leader while source and destination overlap.
        self.killed_leader = self.leader_id
        return self.killed_leader

    def after_recovery(self):
        self.phase = "ledger-transfer-after-leader-loss"
        self.observe("source", 2)
        self.observe("destination", 1)
        confirmed = self.mutate(4, self.token("source") + self.owner("destination", 1), changed=False)
        require(confirmed["revision"] == self.share["revision"]
                and int(confirmed["confirmation_index"]) > int(self.share["mutation_index"]),
                "recovered ownership retry preserves the original revision")
        self.mutate(3, self.token("destination"))
        self.mutate(5, self.token("source") + self.token("destination"))
        released = self.mutate(6, self.token("source"))
        self.observe("source", 4)
        self.observe("destination", 2)
        self.wait("final ownership caught up on every voter", lambda: self.through(int(released["mutation_index"])))
        self.save("ledger-after-status.json", self.statuses())
        self.save("ledger-history.json", self.ledger_history)
        self.save("ledger-acceptance.json", {"complete": True,
            "killed_agreed_leader": self.killed_leader, "final_leader": self.leader_id,
            "source_released": True, "destination_published": True,
            "exact_owner_bytes_checked": True, "duplicate_confirmation_checked": True,
            "share_receipt": self.share, "release_receipt": released,
            "requests": len(self.ledger_history),
            "scope": "Explicit registered SST ownership survives one actual leader container death and remote-checkpoint recovery. All replicas reach the final receipt. No production backfill, reader drainage, physical deletion, all-voter fault matrix or full21 claim."})

    def public_manifest(self):
        if self.ledger_history and not (self.out / "ledger-history.json").exists():
            if not (self.out / "failed-ledger-history.json").exists():
                self.save("failed-ledger-history.json", self.ledger_history)
        super().public_manifest()


if __name__ == "__main__":
    base.Cell = RetentionCell
    base.main()
