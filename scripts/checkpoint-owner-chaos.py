#!/usr/bin/env python3
"""Observe automatic checkpoint owners across the original bounded leader kill.

This default-feature cell never submits a retention mutation. An acquired-owner
pre-upload crash needs the separately qualified checkpoint-testing owned gate.
"""
import hashlib
import importlib.util
import json
from pathlib import Path
import re
import struct
import time

from wal_layout import checkpoint_manifest

spec = importlib.util.spec_from_file_location(
    "checkpoint_cell", Path(__file__).with_name("checkpoint-publication-chaos.py"))
base = importlib.util.module_from_spec(spec)
spec.loader.exec_module(base)
require = base.require
MAGIC = b"KV9CHECKPOINT\x01"
REQUIRED_SOURCES = (
    "crates/server/src/checkpoint_retention.rs",
    "crates/server/src/remote_storage.rs",
    "crates/engine/src/flush_journal.rs",
    "crates/engine/src/checkpoint.rs",
    "crates/meta/src/retention.rs",
    "crates/common/src/root.rs",
)


def sealed(body):
    return body + hashlib.sha256(body).digest()


def derive(root, manifest_bytes, predecessor):
    """Independent literal encoding of CheckpointOwners::new, not topology math."""
    require(root.startswith(b"KV9ROOT\0") and len(root) >= 26, "root descriptor")
    require(manifest_bytes.startswith(MAGIC) and len(manifest_bytes) <= 1024**2,
            "bounded checkpoint descriptor")
    require(type(predecessor) is int and 0 <= predecessor < 2**64,
            "actual predecessor generation")
    manifest = checkpoint_manifest(manifest_bytes)
    scope = manifest["scope"]
    require(scope["cluster"] == root[10:26].hex() and scope["region"] == 1,
            "checkpoint root/region scope")
    # Preserve serde's specified field order; do not hash a sorted-key rewrite.
    canonical = {"scope": {k: scope[k] for k in ("cluster", "region", "conf_ver", "version")},
                 "term": manifest["term"], "index": manifest["index"],
                 "files": [{k: f[k] for k in
                            ("key", "sha256", "cf", "smallest", "largest", "size", "count")}
                           for f in manifest["files"]]}
    require(MAGIC + json.dumps(canonical, ensure_ascii=False, separators=(",", ":")).encode()
            == manifest_bytes, "exact canonical manifest bytes")
    digest = hashlib.sha256(root).digest()
    operation = hashlib.sha256(b"kv9-manifest-v1" + struct.pack(">Q", predecessor)
                               + manifest_bytes).digest()
    resources = []
    for file in manifest["files"]:
        require(re.fullmatch("[0-9a-f]{64}", file["sha256"]) is not None,
                "canonical SST hash")
        resources.append(b"\x01" + hashlib.sha256(file["key"].encode()).digest()[:16]
                         + bytes.fromhex(file["sha256"]))
    resources.sort(key=lambda row: row[1:17])
    require(resources and len({r[1:17] for r in resources}) == len(resources),
            "nonempty distinct automatic resource closure")
    return {"root_digest": digest.hex(), "manifest": manifest,
            "manifest_sha256": hashlib.sha256(manifest_bytes).hexdigest(),
            "predecessor_generation": predecessor, "operation": operation.hex(),
            "resources": [r.hex() for r in resources],
            "owner_ids": {name: hashlib.sha256(label + digest + operation).digest()[:16].hex()
                          for name, label in (("pending", b"kv9-checkpoint-pending-v1"),
                                              ("version", b"kv9-checkpoint-version-v1"))}}


def owner_bytes(binding, name, phase):
    scope = binding["manifest"]["scope"]
    body = (b"KV9OWN01" + bytes.fromhex(binding["root_digest"])
            + bytes.fromhex(binding["owner_ids"][name])
            + bytes([{"pending": 2, "version": 3}[name]])
            + struct.pack(">QQQ", scope["region"], scope["conf_ver"], scope["version"])
            + bytes.fromhex(binding["operation"]) + bytes.fromhex(binding["manifest_sha256"])
            + b"\0\0" + struct.pack(">QB", 1, phase)
            + struct.pack(">I", len(binding["resources"]))
            + b"".join(bytes.fromhex(r) for r in binding["resources"]))
    require(len(body) + 32 <= 16 * 1024, "canonical owner observation bound")
    return sealed(body)


class OwnerCell(base.Cell):
    def __init__(self, args):
        super().__init__(args)
        self.owner_history = []
        self.binding = None

    def run(self):
        inputs = json.loads(self.args.inputs.read_text())
        sources = inputs.get("source_pins", inputs.get("sources", {}))
        require(all(name in sources for name in REQUIRED_SOURCES),
                "runtime inputs must include automatic-owner/journal source pins")
        (self.out / "checkpoint-owner-chaos.py").write_bytes(Path(__file__).read_bytes())
        self.save("owner-helper-binding.json", {
            "helper": base.pin(__file__),
            "base_helper": base.pin(Path(__file__).with_name("checkpoint-publication-chaos.py")),
            "required_sources": {name: sources[name] for name in REQUIRED_SOURCES},
            "retention_mutations_by_fixture": 0,
            "scope": "Default worker publication and one actual leader death; no testing gate enabled."})
        super().run()

    def choose_victim(self):
        self.killed_leader = self.leader_id
        return self.killed_leader

    def read_owner(self, name):
        for attempt in range(8):
            require(len(self.owner_history) < 512, "bounded owner observation history")
            node = self.leader_id
            args = ["--root-digest", self.binding["root_digest"], "--owner-id",
                    self.binding["owner_ids"][name]]
            started = time.time_ns()
            raw, code = self.exec(self.pods[node], "/usr/local/bin/kv9", "client",
                                  "retention-owner", "--addr", self.services[node] + ":20160",
                                  *args, allowed=(0, 1))
            stderr = (self.out / "commands" / f"{self.count:05d}" / "stderr").read_text()
            self.owner_history.append({"phase": self.phase, "name": name, "node": node,
                "args": args, "attempt": attempt, "started_ns": started,
                "ended_ns": time.time_ns(), "command_number": self.count,
                "exit_code": code, "output": raw.decode()})
            if code == 0:
                pairs = [line.split("=", 1) for line in raw.decode().splitlines()]
                require(all(len(p) == 2 for p in pairs), "typed owner output")
                result = dict(pairs)
                require(len(result) == len(pairs), "duplicate owner output field")
                return result
            require("not_leader=true" in stderr + raw.decode(), "unknown owner read; no retry")
            self.wait("owner observation leader after typed refusal", self.agreement)
        raise RuntimeError("owner typed leader retry bound")

    def settled(self):
        phases = {"pending": (1, 2, 3, 4), "version": (1, 2)}
        observed = {}
        for name in phases:
            result = self.read_owner(name)
            if result == {"found": "false"}:
                observed[name] = None
                continue
            matches = [phase for phase in phases[name]
                       if result == {"found": "true", "owner_hex": owner_bytes(self.binding, name, phase).hex()}]
            require(len(matches) == 1, "exact automatic owner binding/closure/phase: " + name)
            observed[name] = matches[0]
        return observed if observed == {"pending": 4, "version": 2} else None

    def layout(self, node, label=None):
        layout = super().layout(node, label)
        if label != "before-fault":
            return layout
        require(node == self.killed_leader and self.binding is None,
                "one selected leader checkpoint binding")
        self.phase = "automatic-owner-selected-checkpoint"
        require(layout.checkpoint_bytes, "actual selected checkpoint")
        manifest_path = self.out / "owner-selected-manifest.bin"
        manifest_path.write_bytes(layout.checkpoint_bytes)
        cut = layout.checkpoint["index"]
        predecessors, log_pins, acquisition = set(), [], []
        for voter in (1, 2, 3):
            raw = self.k("logs", "-n", self.namespace, self.pods[voter], "-c", "kv9")[0]
            path = self.out / f"owner-before-n{voter}.log"
            path.write_bytes(raw)
            log_pins.append(base.pin(path))
            for n, generation, through in re.findall(
                    rb"node (\d+) checkpoint pending generation=(\d+) through=(\d+) resuming", raw):
                if int(n) == voter and int(through) == cut:
                    predecessors.add(int(generation))
            acquisition.extend(raw.decode().splitlines())
        require(len(predecessors) == 1, "unique actual worker predecessor for selected cut")
        self.binding = derive((self.out / "root.bin").read_bytes(), layout.checkpoint_bytes,
                              predecessors.pop())
        expected = "checkpoint through=" + str(cut) + " owners pending=OwnerId(" + str(
            list(bytes.fromhex(self.binding["owner_ids"]["pending"]))) + ") version=OwnerId(" + str(
            list(bytes.fromhex(self.binding["owner_ids"]["version"]))) + ") confirmed before remote I/O"
        require(any(expected in line for line in acquisition),
                "actual worker pre-I/O owner IDs agree with independent derivation")
        self.binding.update({"selected_manifest": base.pin(manifest_path),
            "selected_topology": base.pin(self.out / f"before-fault-n{node}.topology.bin"),
            "worker_logs": log_pins, "predecessor_authority": "Unique selected-cut worker log; independently checked against successful recovery publication generation after death."})
        self.save("owner-selected-binding.json", self.binding)
        self.wait("automatic selected Pending released and Version published", self.settled)
        # Refuse if the worker advanced the recovery selection during observation.
        current = super().layout(node)
        require(current.checkpoint_bytes == layout.checkpoint_bytes,
                "selected checkpoint changed during owner observation")
        self.save("owner-before-status.json", self.statuses())
        return layout

    def after_recovery(self):
        self.phase = "automatic-owner-after-leader-recovery"
        logs = (self.out / "recovered-process.log").read_bytes()
        matches = [[int(x) for x in row] for row in re.findall(
            rb"node (\d+) recovered checkpoint generation=(\d+) cut=(\d+) publication=(\d+)", logs)]
        exact = [row for row in matches if row[0] == self.killed_leader
                 and row[1] == self.binding["predecessor_generation"] + 1
                 and row[2] == self.binding["manifest"]["index"] and row[3] > row[2]]
        require(exact, "actual recovered publication binds selected predecessor generation")
        require(self.settled(), "exact automatic owners survive leader death")
        selected = json.loads((self.out / "selected-sst-inventory.json").read_text())
        require(selected["complete"] and [r["reference"] for r in selected["files"]]
                == self.binding["manifest"]["files"], "captured bytes cover exact owner closure")
        rows = self.statuses()
        require(rows and set(rows) == {1, 2, 3} and self.agreement(rows), "complete stable status")
        barrier = int(rows[self.leader_id]["driver_applied_index"])
        def caught_up():
            now = self.statuses()
            return now and set(now) == {1, 2, 3} and self.agreement(now) and all(
                int(row["driver_applied_index"]) >= barrier for row in now.values())
        self.wait("all voters beyond automatic owner read barrier", caught_up)
        self.save("owner-after-status.json", self.statuses())
        self.save("owner-history.json", self.owner_history)
        self.save("owner-acceptance.json", {"complete": True,
            "binding": base.pin(self.out / "owner-selected-binding.json"),
            "history": base.pin(self.out / "owner-history.json"),
            "selected_sst_inventory": base.pin(self.out / "selected-sst-inventory.json"),
            "killed_agreed_leader": self.killed_leader, "recovery_publication": exact,
            "pending_phase": "Released", "version_phase": "Published",
            "all_voter_barrier_index": barrier, "retention_mutations_by_fixture": 0,
            "live_acquired_phase_observed": False, "pre_upload_crash_exercised": False,
            "scope": "Default worker deterministic ownership and exact selected SST closure survive one leader container death. Released implies the source state machine accepted acquisition/transfer; no live transient acquisition or pre-upload crash claim."})

    def k(self, *args, **kw):
        if args == ("scale", "deployment", "--all", "-n", self.namespace, "--replicas=0"):
            for n in (1, 2, 3):
                raw = super().k("logs", "-n", self.namespace, self.pods[n], "-c", "kv9")[0]
                (self.out / f"owner-final-n{n}.log").write_bytes(raw)
        return super().k(*args, **kw)

    def public_manifest(self):
        if self.owner_history and not (self.out / "owner-history.json").exists():
            if not (self.out / "failed-owner-history.json").exists():
                self.save("failed-owner-history.json", self.owner_history)
        if not (self.out / "owner-protocol-payloads.json").exists():
            files = sorted(self.out.glob("selected-sst-*.bin")) + [
                self.out / f"n{n}-stopped-data.tar" for n in (1, 2, 3)
                if (self.out / f"n{n}-stopped-data.tar").exists()]
            self.save("owner-protocol-payloads.json", {
                "runtime_complete": (self.out / "result.json").exists(),
                "files": [base.pin(p) for p in files],
                "scope": "Explicit separate full object and three stopped-store protocol archive selector; credentials, CA, images and executables excluded. Missing files on failure remain missing."})
        super().public_manifest()


if __name__ == "__main__":
    base.Cell = OwnerCell
    base.main()
