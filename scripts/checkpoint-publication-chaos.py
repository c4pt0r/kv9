#!/usr/bin/env python3
"""One bounded, actual PodChaos restart from a published remote checkpoint.

Uses an already-tested binary. This is HTTP/remote-checkpoint plus local-store
process recovery, not object-store power-loss or full Chaos/performance coverage.
"""
import argparse
import ctypes
import hashlib
import json
import os
from pathlib import Path
import re
import secrets
import shutil
import signal
import stat
import subprocess
import sys
import tarfile
import time

from wal_layout import decode_topology

GIB = 1024**3
MIB = 1024**2
MINIO = "sha256:69b2ec208575b69597784255eec6fa6a2985ee9e1a47f4411a51f7f5fdd193a9"
MINIO_TAG = "quay.io/minio/minio:latest"
BASE = "sha256:b2b7ea366714195a1e1c5b2b578ece85c0b3920381a8654d038d9684f009613c"
KIND = os.environ.get("KV9_KIND", "/tmp/kv9-p0-tools/kind-linux-amd64")
CLUSTER = "kv9-chaos-ci-p0-20260908"
KUBECONFIG = os.environ.get("KV9_CHAOS_KUBECONFIG", "/tmp/kv9-p0-ci-chaos.kubeconfig")
PROTECTED = {
    "chaos-mesh": "10165ce0-6d25-4c22-8f36-1e5083434ced",
    "default": "ef9282ce-08cb-4362-b933-edd43ec0e44e",
    "kube-node-lease": "e0bc942a-0ee9-4485-a880-abf28ff26671",
    "kube-public": "dca2e055-1478-4562-8a3c-1f1f60cadfb8",
    "kube-system": "905986f0-a2df-4651-adab-5a363093cc78",
    "kv9-chaos-1788964167-3930517": "f7ac3bbd-584c-423e-90c5-630dc18338aa",
    "kv9-chaos-1788967535-626870": "ea5cc740-2cd7-45f0-9b6b-b558600a9425",
    "local-path-storage": "870737bc-5c6c-4675-8327-2c30d523d5a7",
}
OLD_FAULT_UID = "c3ededa7-0746-4864-bd95-4425ceb3d663"


def require(ok, message):
    if not ok:
        raise RuntimeError(message)


def pin(path):
    path = Path(path)
    with path.open("rb") as f:
        digest = hashlib.file_digest(f, "sha256").hexdigest()
    return {"path": str(path), "bytes": path.stat().st_size, "sha256": digest}


def parent_death(parent):
    require(ctypes.CDLL(None).prctl(1, signal.SIGKILL, 0, 0, 0) == 0, "parent-death setup")
    if os.getppid() != parent:
        os.kill(os.getpid(), signal.SIGKILL)


class FilesystemBudget:
    """Charge each distinct host/output filesystem's retained high-water decrease."""

    def __init__(self, output, *, stat_fn=None, statvfs_fn=None):
        self.stat = stat_fn or os.stat
        self.statvfs = statvfs_fn or os.statvfs
        self.paths = {"host": Path("/"), "output": Path(output)}
        self.devices = {role: self.stat(path).st_dev for role, path in self.paths.items()}
        self.filesystems = {}
        for role, device in self.devices.items():
            if device not in self.filesystems:
                v = self.statvfs(self.paths[role])
                free = v.f_bavail * v.f_frsize
                self.filesystems[device] = {"device": device, "roles": [], "paths": [],
                    "baseline_available_bytes": free, "current_available_bytes": free,
                    "minimum_available_bytes": free, "maximum_observed_decrease_bytes": 0}
            self.filesystems[device]["roles"].append(role)
            self.filesystems[device]["paths"].append(str(self.paths[role]))

    @property
    def host(self):
        return self.filesystems[self.devices["host"]]

    def sample(self):
        for role, path in self.paths.items():
            require(self.stat(path).st_dev == self.devices[role], "filesystem device changed: " + role)
        for row in self.filesystems.values():
            v = self.statvfs(row["paths"][0])
            free = v.f_bavail * v.f_frsize
            row["current_available_bytes"] = free
            row["minimum_available_bytes"] = min(row["minimum_available_bytes"], free)
            row["maximum_observed_decrease_bytes"] = max(0, row["baseline_available_bytes"] - row["minimum_available_bytes"])
        return self.snapshot()

    def snapshot(self):
        return {"schema_version": 1, "host_device": self.devices["host"],
                "output_device": self.devices["output"],
                "filesystems": [dict(row, roles=list(row["roles"]), paths=list(row["paths"]))
                                for row in self.filesystems.values()],
                "maximum_observed_total_decrease_bytes": sum(
                    row["maximum_observed_decrease_bytes"] for row in self.filesystems.values())}

    def check(self, *, launch=False):
        for row in self.filesystems.values():
            roles = "/".join(row["roles"])
            if launch:
                require(row["baseline_available_bytes"] >= 9*GIB + 8*MIB,
                        "9 GiB plus 8 MiB launch headroom: " + roles)
            require(row["current_available_bytes"] >= 8*GIB, "8 GiB available floor: " + roles)
        require(self.snapshot()["maximum_observed_total_decrease_bytes"] <= GIB,
                "1 GiB total conservative filesystem decrease")

    def inherit(self, failure, prior):
        """Retain failed-attempt charges; refuse a filesystem switch during reuse."""
        require(self.stat(prior).st_dev == self.devices["output"], "prior output filesystem changed")
        old = failure.get("filesystem_budget")
        if old is None:
            require(len(self.filesystems) == 1, "legacy failure needs the same host/output filesystem")
            prior_rows = [dict(device=self.devices["host"], roles=["host", "output"],
                              baseline_available_bytes=failure["baseline_available_bytes"],
                              minimum_available_bytes=failure["minimum_available_bytes"])]
        else:
            require(old["schema_version"] == 1 and old["host_device"] == self.devices["host"]
                    and old["output_device"] == self.devices["output"], "prior filesystem budget binding")
            prior_rows = old["filesystems"]
        require(len(prior_rows) == len(self.filesystems)
                and {row["device"] for row in prior_rows} == set(self.filesystems), "prior filesystem set")
        for prior_row in prior_rows:
            row = self.filesystems[prior_row["device"]]
            require(prior_row["roles"] == row["roles"], "prior filesystem roles")
            baseline, minimum = prior_row["baseline_available_bytes"], prior_row["minimum_available_bytes"]
            require(type(baseline) is int and type(minimum) is int and 0 <= minimum <= baseline,
                    "prior filesystem counters")
            row["baseline_available_bytes"] = baseline
            row["minimum_available_bytes"] = min(minimum, row["current_available_bytes"])
        self.sample()


class Cell:
    def __init__(self, args):
        self.args = args
        self.out = args.output.resolve()
        self.out.mkdir(mode=0o700)
        (self.out / "commands").mkdir()
        self.namespace = "kv9-c04-checkpoint-" + secrets.token_hex(5)
        self.image = "kv9-c04-checkpoint:" + self.namespace
        self.bucket = "checkpoint-" + secrets.token_hex(5)
        self.values = {k: secrets.token_hex(24) for k in ("bootstrap", "cluster", "client", "access", "secret")}
        self.env = {**os.environ, "KUBECONFIG": KUBECONFIG, "KV9_BOOTSTRAP_TOKEN": self.values["bootstrap"]}
        self.started = time.monotonic()
        self.storage = FilesystemBudget(self.out)
        self.prior = None
        if args.prior_preparation:
            self.prior = args.prior_preparation.resolve()
            failure = json.loads((self.prior / "failure.json").read_text())
            require(failure["complete"] is False and failure["phase"] == "provision" and failure["fault_uid"] is None, "only a failed pre-workload provision may supply images/baseline")
            require(not (self.prior / "failed-history.json").exists(), "prior workload must not have started")
            self.storage.inherit(failure, self.prior)
        self.image_failure = None
        if args.image_preparation_failure:
            require(args.retained_ca_base and not args.prior_preparation, "failed image preparation requires the new-binary CA route")
            self.image_failure = args.image_preparation_failure.resolve()
            failure = json.loads((self.image_failure/"failure.json").read_text())
            require(failure["complete"] is False and failure["phase"] == "image-load" and failure["namespace_uid"] is None and failure["fault_uid"] is None, "only a pre-fixture image failure may preserve its budget")
            require(not (self.image_failure/"failed-history.json").exists() and not (self.image_failure/"namespace.json").exists(), "prior workload/fault forbidden")
            binding = json.loads((self.image_failure/"source-binding.json").read_text())
            require(binding["binary"] == pin(args.binary) and binding["binary"]["sha256"] == args.binary_sha256 and binding["inputs"] == pin(args.inputs), "failed image's exact new binary and input authority")
            require(pin(self.image_failure/"image-context/kv9")["sha256"] == args.binary_sha256, "preserved executable context identity")
            self.storage.inherit(failure, self.image_failure)
        self.baseline = self.storage.host["baseline_available_bytes"]
        self.minimum = self.storage.host["minimum_available_bytes"]
        self.count = 0
        self.ns_uid = None
        self.fault_uid = None
        self.child = None
        self.services = {}
        self.pods = {}
        self.history = []
        self.leader_id = None
        self.phase = "preflight"
        with open(self.out / "credentials.json", "x", opener=lambda p, f: os.open(p, f, 0o600)) as f:
            json.dump(self.values, f)

    def save(self, name, value):
        path = self.out / name
        path.parent.mkdir(parents=True, exist_ok=True)
        with path.open("x") as f:
            json.dump(value, f, indent=2, sort_keys=True)
            f.write("\n")
            f.flush()
            os.fsync(f.fileno())

    def available(self):
        # Legacy scalar fields describe the Docker/Kind host filesystem.
        v = os.statvfs("/")
        return v.f_bavail * v.f_frsize

    def guard(self):
        storage = self.storage.sample()
        free = self.storage.host["current_available_bytes"]
        self.minimum = self.storage.host["minimum_available_bytes"]
        self.storage.check()
        require(time.monotonic() - self.started < 1200, "1200-second outer deadline")
        with (self.out / "resources.jsonl").open("a") as f:
            f.write(json.dumps({"observed_ns": time.time_ns(), "phase": self.phase, "available_bytes": free,
                                "decrease_from_original_baseline": max(0, self.baseline - free),
                                "filesystem_budget": storage}) + "\n")

    def redact(self, value):
        for secret in self.values.values():
            value = value.replace(secret.encode(), b"<REDACTED>")
        return value

    def command(self, argv, *, stdin=None, sensitive=False, payload=False, allowed=(0,), seconds=45, cap=4*MIB):
        self.guard()
        self.count += 1
        d = self.out / "commands" / f"{self.count:05d}"
        d.mkdir()
        self.save(str(d.relative_to(self.out) / "invocation.json"), {
            "argv": [str(x) for x in argv], "started_ns": time.time_ns(), "timeout_seconds": seconds,
            "stdout_is_payload": payload,
            "stdin": "secret omitted" if sensitive else (None if stdin is None else {"bytes": len(stdin), "sha256": hashlib.sha256(stdin).hexdigest()}),
        })
        parent = os.getpid()
        source = d / "stdin"
        if stdin is not None:
            with open(source, "xb", opener=lambda p, f: os.open(p, f, 0o600)) as f:
                f.write(stdin)
        try:
            with (source.open("rb") if stdin is not None else open(os.devnull, "rb")) as inp, (d / "stdout").open("xb") as out, (d / "stderr").open("xb") as err:
                child = subprocess.Popen([str(x) for x in argv], env=self.env, stdin=inp, stdout=out, stderr=err,
                                         start_new_session=True, preexec_fn=lambda: parent_death(parent))
                self.child = child
                raw = Path(f"/proc/{child.pid}/stat").read_text(); fields = raw[raw.rfind(")")+2:].split()
                birth = {"pid": child.pid, "start_ticks": int(fields[19]), "boot_id": Path("/proc/sys/kernel/random/boot_id").read_text().strip()}
                self.save(str(d.relative_to(self.out) / "child-start.json"), birth)
                began = time.monotonic()
                while child.poll() is None:
                    self.guard()
                    require(time.monotonic() - began <= seconds, "owned command deadline")
                    require((d / "stdout").stat().st_size <= cap and (d / "stderr").stat().st_size <= MIB, "owned command output bound")
                    time.sleep(.1)
                code = child.wait()
            stdout = (d / "stdout").read_bytes(); stderr = (d / "stderr").read_bytes()
            for name, content in (("stdout", stdout), ("stderr", stderr)):
                clean = self.redact(content)
                if clean != content:
                    (d / name).write_bytes(clean)
            self.save(str(d.relative_to(self.out) / "terminal.json"), {"complete": True, "exit_code": code, "child_reaped": True, **birth,
                "ended_ns": time.time_ns(), "stdout": pin(d / "stdout"), "stderr": pin(d / "stderr")})
            require(code in allowed, f"command {self.count} failed with exit {code}: {self.redact(stderr)[-1500:].decode(errors='replace')}")
            return stdout, code
        finally:
            if self.child is not None and self.child.poll() is None:
                os.killpg(self.child.pid, signal.SIGKILL)
                self.child.wait()
            self.child = None
            if sensitive and source.exists():
                source.unlink()  # Only the temporary Secret transport; original0600 credentials remain.

    def k(self, *args, **kw):
        return self.command(["kubectl", "--kubeconfig", KUBECONFIG, "--request-timeout=30s", *args], **kw)

    def obj(self, *args):
        return json.loads(self.k(*args, "-o", "json")[0])

    def apply(self, obj, *, sensitive=False):
        if not sensitive:
            self.save(f"manifests/{obj['kind']}-{obj['metadata']['name']}.json", obj)
        self.k("apply", "-f", "-", stdin=json.dumps(obj).encode(), sensitive=sensitive)

    def wait(self, label, function, seconds=60):
        end = time.monotonic() + seconds
        while time.monotonic() < end:
            self.guard()
            result = function()
            if result:
                return result
            time.sleep(.3)
        raise RuntimeError("deadline: " + label)

    def exec(self, pod, *argv, **kw):
        return self.k("exec", "-n", self.namespace, pod, "--", *argv, **kw)

    def pod(self, node):
        pods = self.obj("get", "pods", "-n", self.namespace, "-l", f"app=kv9,kv9-node={node}")["items"]
        found = [p for p in pods if p["status"].get("phase") == "Running" and not p["metadata"].get("deletionTimestamp")]
        require(len(found) == 1, "exact running voter Pod")
        return found[0]

    def statuses(self):
        rows = {}
        for n in (1, 2, 3):
            raw, code = self.exec(self.pods[n], "cat", "/data/status", allowed=(0, 1))
            if code:
                return None
            rows[n] = dict(line.split("=", 1) for line in raw.decode().splitlines())
        return rows

    def agreement(self, rows=None):
        rows = self.statuses() if rows is None else rows
        if not rows or any(r.get("bootstrap_state") != "Serving" or r.get("fatal") for r in rows.values()):
            return None
        ids = {r.get("leader_id") for r in rows.values()}
        if len(ids) != 1 or not ids <= {"1", "2", "3"}:
            return None
        n = int(next(iter(ids)))
        if rows[n].get("role") != "leader":
            return None
        self.leader_id = n
        return rows

    def client(self, verb, *args):
        for attempt in range(8):
            n = self.leader_id
            start = time.time_ns()
            out, code = self.exec(self.pods[n], "/usr/local/bin/kv9", "client", verb, "--addr", self.services[n]+":20160", *args, allowed=(0, 1))
            self.history.append({"phase": self.phase, "verb": verb, "args": list(args), "attempt": attempt,
                                 "started_ns": start, "ended_ns": time.time_ns(), "exit_code": code, "output": out.decode(errors="replace")})
            if code == 0:
                return out.decode().strip()
            # Never retry ambiguous writes; only an explicit pre-append NotLeader.
            stderr = (self.out / "commands" / f"{self.count:05d}" / "stderr").read_text()
            require("not_leader=true" in stderr + out.decode(errors="replace"), "nonretryable/ambiguous client failure")
            self.wait("leader after typed refusal", self.agreement)
        raise RuntimeError("typed leader retry bound")

    def put(self, key, value):
        out = self.client("raw-put", "--keyspace", self.keyspace, "--key-hex", key.hex(), "--value-hex", value.hex())
        return dict(line.split("=", 1) for line in out.splitlines())

    def check(self, key, value):
        out = self.client("raw-get", "--keyspace", self.keyspace, "--key-hex", key.hex())
        require(out == ("found=false" if value is None else "value_hex="+value.hex()), "exact public KV readback")

    def layout(self, node, label=None):
        raw = self.exec(self.pods[node], "cat", "/data/catalog.wal")[0]
        layout = decode_topology(raw, Path("/data"))
        if label:
            path = self.out / f"{label}-n{node}.topology.bin"
            path.write_bytes(raw)
            self.save(f"{label}-n{node}.json", layout.evidence())
        return layout

    def protection(self):
        namespaces = self.obj("get", "namespaces")["items"]
        observed = {n["metadata"]["name"]: n["metadata"]["uid"] for n in namespaces}
        require(all(observed.get(n) == uid for n, uid in PROTECTED.items()), "historical namespace UID changed")
        fault = self.obj("get", "podchaos", "store-loss-kill", "-n", "kv9-chaos-1788967535-626870")
        require(fault["metadata"]["uid"] == OLD_FAULT_UID, "historical fault identity changed")
        return {"namespaces": {n: observed[n] for n in PROTECTED}, "fault": fault}

    def public_manifest(self):
        excluded = {"credentials.json", "public-evidence-manifest.json", "image-context/kv9"}
        for folder in (self.out / "commands").iterdir():
            invocation = folder / "invocation.json"
            if invocation.is_file():
                row = json.loads(invocation.read_text())
                if row.get("stdout_is_payload"):
                    excluded.add(str((folder / "stdout").relative_to(self.out)))
        rows, omitted = [], []
        for path in sorted(self.out.rglob("*")):
            if not path.is_file():
                continue
            name = str(path.relative_to(self.out))
            if name in excluded or name == "runtime-image-context/ca-certificates.crt" or path.suffix == ".tar" or (path.name.startswith("selected-sst-") and path.suffix == ".bin"):
                omitted.append({"path": str(path), "reason": "private credentials, executable/image input or retained payload; not portable metadata"})
                continue
            require(not path.is_symlink(), "public evidence symlink")
            rows.append(dict(relative_path=name, **pin(path)))
        self.save("public-evidence-manifest.json", {"complete": True, "files": rows, "excluded": omitted,
                    "scope": "Nonsecret exact metadata/source/command evidence only; credentials never hashed or archived."})

    def before_fault(self):
        """Extension point after reclamation and before selecting the crash cut."""

    def choose_victim(self):
        return 3 if self.leader_id != 3 else 2

    def after_recovery(self):
        """Extension point after fresh all-voter progress, before scoped cleanup."""

    def run(self):
        require(os.geteuid() == 0, "root needed for isolated Kind/Docker fixture")
        self.guard()
        self.storage.check(launch=True)
        binary = pin(self.args.binary)
        require(binary["sha256"] == self.args.binary_sha256, "tested binary changed")
        inputs = json.loads(self.args.inputs.read_text())
        require(inputs["binary_sha256"] == binary["sha256"], "runtime input binding")
        repo = Path(__file__).resolve().parent.parent
        source_keys = [key for key in ("sources", "source_pins") if key in inputs]
        require(len(source_keys) == 1 and isinstance(inputs[source_keys[0]], dict) and inputs[source_keys[0]], "one exact nonempty source pin map")
        if "binary_bytes" in inputs:
            require(inputs["binary_bytes"] == binary["bytes"], "runtime binary size binding")
        for name, expected in inputs[source_keys[0]].items():
            require(pin(repo/name)["sha256"] == expected, "tested source binding changed: "+name)
        self.save("source-binding.json", {"binary": binary, "inputs": pin(self.args.inputs), "helper": pin(__file__),
                                         "wal_layout": pin(Path(__file__).with_name("wal_layout.py")), "scope": "one remote-checkpoint PodChaos cell"})
        (self.out / "sources").mkdir()
        for source in [Path(__file__), Path(__file__).with_name("wal_layout.py"), Path(__file__).with_name("workload_report.py")]:
            shutil.copyfile(source, self.out / "sources" / source.name)
        shutil.copyfile(self.args.inputs, self.out / "runtime-inputs.json")
        self.save("protected-before.json", self.protection())
        nodes = self.obj("get", "nodes")["items"]
        require([n["metadata"]["name"] for n in nodes] == [CLUSTER+"-control-plane"], "exact isolated Kind cluster")
        self.k("get", "crd", "podchaos.chaos-mesh.org")
        for image in (BASE, MINIO):
            require(self.command(["docker", "image", "inspect", image, "--format", "{{.Id}}"])[0].decode().strip() == image, "retained image absent")
        self.phase = "image-load"
        if self.args.retained_ca_base:
            require(self.prior is None, "new binary image route cannot reuse failed-attempt binary authority")
            inherited = Path("/tmp/kv9-c04-checkpoint-publication-chaos-20260915-fourth")
            prior_image = json.loads((inherited/"images.json").read_text())
            lineage = json.loads((inherited/"runtime-ca-lineage.json").read_text())
            parent_image = "sha256:4c4a246d9fd6d00eafa71382158e4395cd7186c420ce37960c17b85abdbec975"
            require(prior_image["kv9_id"] == lineage["image_id"] == parent_image, "retained CA-bearing image lineage")
            require(self.command(["docker","image","inspect",parent_image,"--format","{{.Id}}"])[0].decode().strip() == parent_image, "retained CA image missing")
            context = self.out/"image-context";context.mkdir()
            if self.image_failure:
                os.link(self.image_failure/"image-context/kv9",context/"kv9")
                self.save("prior-image-preparation.json",{"failure":pin(self.image_failure/"failure.json"),"tool_terminal":pin(self.image_failure/"tool-terminal.json"),"source_binding":pin(self.image_failure/"source-binding.json"),"actual_source":pin(self.image_failure/"sources/checkpoint-publication-chaos.py"),"original_baseline_available_bytes":self.baseline,"context_reused_by_hardlink":True,"no_workload_replayed":True})
            else:
                shutil.copyfile(self.args.binary,context/"kv9")
                (context/"kv9").chmod(0o755)
            require(pin(context/"kv9")["sha256"] == binary["sha256"], "new image executable copy")
            parent_tag = prior_image["kv9_tag"]
            require(self.command(["docker","image","inspect",parent_tag,"--format","{{.Id}}"])[0].decode().strip() == parent_image, "retained CA image tag changed")
            (context/"Dockerfile").write_text('FROM '+parent_tag+'\nCOPY kv9 /usr/local/bin/kv9\n')
            self.command(["docker","build","--pull=false","--network=none","-t",self.image,str(context)],seconds=180)
            image_id = self.command(["docker","image","inspect",self.image,"--format","{{.Id}}"])[0].decode().strip()
            self.command([KIND,"load","docker-image","--name",CLUSTER,self.image],seconds=180)
            self.save("runtime-ca-lineage.json",{"parent_id":parent_image,"retained_image_authority":pin(inherited/"images.json"),"retained_ca_authority":pin(inherited/"runtime-ca-lineage.json"),"image":self.image,"image_id":image_id,"new_binary":binary,"ca_input":lineage["ca_input"],"new_image_contains_new_binary_only_over_retained_ca_base":True,"ca_bytes_excluded_from_public_evidence":True})
        elif self.prior:
            prior = json.loads((self.prior/"images.json").read_text())
            cleanup = json.loads((self.prior/"failure-post-first/result.json").read_text())
            require(cleanup["complete"] and cleanup["namespace_absent"] and cleanup["no_kv_deployment_or_fault_executed"], "prior preparation scoped cleanup must be complete")
            require(json.loads((self.prior/"source-binding.json").read_text())["binary"] == binary, "prior tested image binary binding")
            require(prior["minio_id"] == MINIO and prior["base_id"] == BASE, "prior image bases")
            self.image = prior["kv9_tag"]
            image_id = self.command(["docker", "image", "inspect", self.image, "--format", "{{.Id}}"])[0].decode().strip()
            require(image_id == prior["kv9_id"], "retained exact previously loaded image")
            require(not any(x["metadata"]["name"] == cleanup["namespace"]["name"] for x in self.obj("get", "namespaces")["items"]), "failed prior namespace still exists")
            self.save("prior-preparation.json", {"failure":pin(self.prior/"failure.json"),"actual_terminal":pin(self.prior/"tool-terminal.json"),"cleanup":pin(self.prior/"failure-post-first/result.json"),"images":pin(self.prior/"images.json"),"original_source":pin(self.prior/"sources/checkpoint-publication-chaos.py"),"original_baseline_available_bytes":self.baseline,"same_images_no_build_or_reload":True})
        else:
            context = self.out / "image-context"; context.mkdir()
            shutil.copyfile(self.args.binary, context/"kv9")
            require(self.command(["docker", "image", "inspect", "ubuntu:24.04", "--format", "{{.Id}}"])[0].decode().strip() == BASE, "retained base tag differs")
            (context/"Dockerfile").write_text('FROM ubuntu:24.04\nCOPY kv9 /usr/local/bin/kv9\nRUN chmod 755 /usr/local/bin/kv9\nENTRYPOINT ["/usr/local/bin/kv9"]\n')
            self.command(["docker", "build", "--pull=false", "--network=none", "-t", self.image, str(context)], seconds=180)
            image_id = self.command(["docker", "image", "inspect", self.image, "--format", "{{.Id}}"])[0].decode().strip()
            require(self.command(["docker", "image", "inspect", MINIO_TAG, "--format", "{{.Id}}"])[0].decode().strip() == MINIO, "retained MinIO tag differs")
            self.command([KIND, "load", "docker-image", "--name", CLUSTER, self.image, MINIO_TAG], seconds=180)
        if not self.args.retained_ca_base:
            # reqwest constructs its native certificate verifier even for this HTTP endpoint.
            # Keep the exact tested executable layer and add only the retained public CA input.
            ca = pin(self.args.ca_bundle)
            require(0 < ca["bytes"] <= MIB, "bounded retained certificate bundle")
            context = self.out/"runtime-image-context";context.mkdir()
            shutil.copyfile(self.args.ca_bundle,context/"ca-certificates.crt")
            parent_image, parent_id = self.image,image_id
            self.image = "kv9-c04-checkpoint:"+self.namespace+"-ca"
            (context/"Dockerfile").write_text('FROM '+parent_image+'\nCOPY ca-certificates.crt /etc/ssl/certs/ca-certificates.crt\n')
            self.command(["docker","build","--pull=false","--network=none","-t",self.image,str(context)],seconds=90)
            image_id = self.command(["docker","image","inspect",self.image,"--format","{{.Id}}"])[0].decode().strip()
            self.command([KIND,"load","docker-image","--name",CLUSTER,self.image],seconds=180)
            self.save("runtime-ca-lineage.json",{"parent_image":parent_image,"parent_id":parent_id,"image":self.image,"image_id":image_id,"ca_input":ca,"binary_unchanged":binary,"ca_bytes_excluded_from_public_evidence":True})
        self.save("images.json", {"kv9_tag": self.image, "kv9_id": image_id, "minio_tag": MINIO_TAG, "minio_id": MINIO, "base_id": BASE})
        self.phase = "provision"
        self.apply({"apiVersion": "v1", "kind": "Namespace", "metadata": {"name": self.namespace, "labels": {"kv9-owner": "c04-checkpoint-publication"}, "annotations": {"chaos-mesh.org/inject": "enabled"}}})
        self.ns_uid = self.obj("get", "namespace", self.namespace)["metadata"]["uid"]
        self.save("namespace.json", {"name": self.namespace, "uid": self.ns_uid})
        self.apply({"apiVersion": "v1", "kind": "Secret", "metadata": {"name": "kv9-auth", "namespace": self.namespace}, "stringData": {
            "bootstrap": self.values["bootstrap"], "cluster": self.values["cluster"], "client": self.values["client"],
            "client-tokens": "admin="+self.values["client"], "access": self.values["access"], "secret": self.values["secret"]}}, sensitive=True)
        def ref(name, key):
            return {"name": name, "valueFrom": {"secretKeyRef": {"name": "kv9-auth", "key": key}}}
        self.apply({"apiVersion": "v1", "kind": "Service", "metadata": {"name": "minio", "namespace": self.namespace}, "spec": {"selector": {"app": "c04-minio"}, "ports": [{"port": 9000, "targetPort": 9000}]}})
        self.apply({"apiVersion": "v1", "kind": "Pod", "metadata": {"name": "minio", "namespace": self.namespace, "labels": {"app": "c04-minio"}}, "spec": {"restartPolicy": "Never", "containers": [{
            "name": "minio", "image": MINIO_TAG, "imagePullPolicy": "Never", "args": ["server", "/data", "--address", ":9000", "--quiet"],
            "env": [ref("MINIO_ROOT_USER", "access"), ref("MINIO_ROOT_PASSWORD", "secret"), {"name": "MINIO_BROWSER", "value": "off"}],
            "resources": {"requests": {"memory": "128Mi", "cpu": "100m"}, "limits": {"memory": "1Gi", "cpu": "2"}},
            "volumeMounts": [{"name": "data", "mountPath": "/data"}], "readinessProbe": {"httpGet": {"path": "/minio/health/live", "port": 9000}, "periodSeconds": 1}}],
            "volumes": [{"name": "data", "emptyDir": {"medium": "Memory", "sizeLimit": "512Mi"}}]}})
        self.k("wait", "-n", self.namespace, "--for=condition=Ready", "pod/minio", "--timeout=60s", seconds=70)
        self.exec("minio", "/bin/sh", "-c", 'export MC_HOST_local="http://${MINIO_ROOT_USER}:${MINIO_ROOT_PASSWORD}@127.0.0.1:9000"; mc mb "local/$1"', "command", self.bucket)
        minio_ip = self.obj("get", "service", "minio", "-n", self.namespace)["spec"]["clusterIP"]
        stores = []
        for n in (1, 2, 3):
            self.apply({"apiVersion": "v1", "kind": "Service", "metadata": {"name": f"kv9-n{n}", "namespace": self.namespace}, "spec": {"selector": {"app": "kv9", "kv9-node": str(n)}, "ports": [{"port": 20160, "targetPort": 20160}]}})
            self.services[n] = self.obj("get", "service", f"kv9-n{n}", "-n", self.namespace)["spec"]["clusterIP"]
            self.apply({"apiVersion": "v1", "kind": "PersistentVolumeClaim", "metadata": {"name": f"data-n{n}", "namespace": self.namespace}, "spec": {"accessModes": ["ReadWriteOnce"], "resources": {"requests": {"storage": "128Mi"}}}})
            self.apply({"apiVersion": "v1", "kind": "Pod", "metadata": {"name": f"prepare-{n}", "namespace": self.namespace, "labels": {"app": "c04-provision"}}, "spec": {"terminationGracePeriodSeconds": 0, "restartPolicy": "Never", "containers": [{"name": "prepare", "image": self.image, "imagePullPolicy": "Never", "command": ["sleep", "3600"], "volumeMounts": [{"name": "data", "mountPath": "/data"}]}], "volumes": [{"name": "data", "persistentVolumeClaim": {"claimName": f"data-n{n}"}}]}})
            self.k("wait", "-n", self.namespace, "--for=condition=Ready", f"pod/prepare-{n}", "--timeout=60s", seconds=70)
            out = self.exec(f"prepare-{n}", "/usr/local/bin/kv9", "store-prepare", "--node-id", str(n), "--data-dir", "/data")[0].decode()
            fields = dict(x.split("=", 1) for x in out.split());require(fields.get("store_prepared") == "true" and re.fullmatch("[0-9a-f]{32}", fields.get("store_incarnation", "")), "actual prepared store")
            stores.append(f"{n}={fields['store_incarnation']}")
        root = self.out / "root.bin"
        self.command([str(self.args.binary), "root-create", "--output", str(root), "--voters", ",".join(f"{n}@{self.services[n]}:20160" for n in (1,2,3)), "--store-incarnations", ",".join(stores)])
        self.k("create", "configmap", "kv9-root", "-n", self.namespace, "--from-file=root.bin="+str(root))
        for n in (1,2,3):
            self.k("delete", "pod", f"prepare-{n}", "-n", self.namespace, "--wait=true", "--timeout=30s")
            environment = [ref("KV9_BOOTSTRAP_TOKEN", "bootstrap"),ref("KV9_CLUSTER_TOKEN", "cluster"),ref("KV9_CLIENT_TOKEN", "client"),ref("KV9_CLIENT_TOKENS", "client-tokens"),ref("KV9_OBJECT_STORE_ACCESS_KEY", "access"),ref("KV9_OBJECT_STORE_SECRET_KEY", "secret"),{"name":"KV9_STORAGE","value":"minio"},{"name":"KV9_OBJECT_STORE_ENDPOINT","value":"http://"+minio_ip+":9000"},{"name":"KV9_OBJECT_STORE_BUCKET","value":self.bucket},{"name":"KV9_FLUSH_INTERVAL_MS","value":"100"}]
            self.apply({"apiVersion":"apps/v1","kind":"Deployment","metadata":{"name":f"kv9-n{n}","namespace":self.namespace},"spec":{"replicas":1,"strategy":{"type":"Recreate"},"selector":{"matchLabels":{"app":"kv9","kv9-node":str(n)}},"template":{"metadata":{"labels":{"app":"kv9","kv9-node":str(n)}},"spec":{"terminationGracePeriodSeconds":5,"containers":[{"name":"kv9","image":self.image,"imagePullPolicy":"Never","command":["/bin/bash","-c"],"args":[f'set -euo pipefail; if [ ! -f /data/kv9-store-identity ]; then /usr/local/bin/kv9 init --root /root/root.bin --node-id {n} --data-dir /data; fi; exec /usr/local/bin/kv9 start --node-id {n} --addr 0.0.0.0:20160 --data-dir /data'],"env":environment,"resources":{"requests":{"memory":"128Mi","cpu":"100m"},"limits":{"memory":"512Mi","cpu":"1"}},"volumeMounts":[{"name":"data","mountPath":"/data"},{"name":"root","mountPath":"/root","readOnly":True}]}],"volumes":[{"name":"data","persistentVolumeClaim":{"claimName":f"data-n{n}"}},{"name":"root","configMap":{"name":"kv9-root"}}]}}}})
        self.k("wait", "-n", self.namespace, "--for=condition=Ready", "pods", "-l", "app=kv9", "--timeout=60s", seconds=70)
        self.pods = {n:self.pod(n)["metadata"]["name"] for n in (1,2,3)}
        for n in (1,2,3):
            actual=self.exec(self.pods[n],"sha256sum","/usr/local/bin/kv9")[0].decode().split()[0]
            require(actual==binary["sha256"],"actual voter binary hash differs")
        self.wait("three Serving voters", self.agreement)
        self.save("baseline-status.json", self.statuses())
        self.phase = "checkpoint-reclamation"
        made = self.client("create-keyspace", "--name", "checkpoint-chaos", "--api-type", "raw")
        self.keyspace = dict(x.split("=",1) for x in made.splitlines())["keyspace_id"]
        self.put(b"keep",b"remote-value");self.put(b"deleted",b"old-value")
        self.client("raw-delete","--keyspace",self.keyspace,"--key-hex",b"deleted".hex())
        self.put(b"overwrite",b"old-value")
        boundary = int(self.put(b"overwrite",b"new-value")["applied_index"])
        self.wait("checkpoint covers acknowledged data", lambda: all((self.layout(n).checkpoint or {}).get("index",0)>=boundary for n in (1,2,3)))
        initial = {n:self.layout(n,"initial") for n in (1,2,3)}
        target_paths = {n:[str(initial[n].segment_path(x)) for x in (*initial[n].closed,initial[n].active)] for n in (1,2,3)}
        count = 0
        for turn in range(300):
            if turn % 10 == 0 and all(self.layout(n).active["sequence"]>initial[n].active["sequence"] for n in (1,2,3)):
                break
            self.put(b"wal-rotation-filler", f"rotation-{turn:04d}:".encode().ljust(60*1024,b"x"));count+=1
        require(all(self.layout(n).active["sequence"]>initial[n].active["sequence"] for n in (1,2,3)), "production16MiB rotation")
        out = self.client("raw-delete","--keyspace",self.keyspace,"--key-hex",b"wal-rotation-filler".hex())
        through = int(dict(x.split("=",1) for x in out.splitlines())["applied_index"])
        self.wait("checkpoint covers filler deletion", lambda: all((self.layout(n).checkpoint or {}).get("index",0)>=through for n in (1,2,3)))
        def reclaimed():
            for n, paths in target_paths.items():
                _,code=self.exec(self.pods[n],"/bin/bash","-c",'for p in "$@"; do test ! -e "$p" || exit 1; done',"check",*paths,allowed=(0,1))
                if code:return False
                current=self.layout(n)
                expected=[str(current.segment_path(x))for x in (*current.closed,current.active)]
                script='''set -uo pipefail
for required in "$@"; do test -f "$required" && test ! -L "$required" || exit 2; done
list=$(mktemp /tmp/c04-wal-paths.XXXXXX) || exit 2
trap 'rm -f "$list"' EXIT
find /data/catalog.segments -type f -name '*.wal' -print0 > "$list" || exit 2
files=(/data/catalog.wal)
while IFS= read -r -d '' f; do files+=("$f"); done < "$list"
(( ${#files[@]} > 1 && ${#files[@]} <= 64 )) || exit 2
for f in "${files[@]}"; do
  test -f "$f" && test ! -L "$f" || exit 2
  grep -aFq -e remote-value -e old-value -e new-value "$f"
  code=$?
  if (( code == 0 )); then exit 1; fi
  if (( code != 1 )); then exit 2; fi
done
printf 'physical_files_checked=%s\\n' "${#files[@]}"
'''
                _,code=self.exec(self.pods[n],"/bin/bash","-c",script,"check",*expected,allowed=(0,1))
                if code:return False
            return True
        self.wait("initial segments gone and absorbed values absent",reclaimed)
        self.save("reclamation.json",{"complete":True,"target_paths":target_paths,"filler_writes":count,"filler_bytes_each":60*1024,"through":through,"physical_absence_observed":True})
        self.check(b"keep",b"remote-value");self.check(b"deleted",None);self.check(b"overwrite",b"new-value");self.check(b"wal-rotation-filler",None)
        self.wait("stable leader before fault",self.agreement)
        self.before_fault()
        self.wait("stable leader after pre-fault extension",self.agreement)
        victim = self.choose_victim()
        before=self.pod(victim);old=before["status"]["containerStatuses"][0]
        lifecycle=self.exec(self.pods[victim],"cat","/data/kv9-store-lifecycle")[0]
        selected=self.layout(victim,"before-fault").checkpoint
        require(selected and selected["index"]>=through,"selected remote checkpoint cut")
        self.save("victim-before.json",before);(self.out/"lifecycle-before.bin").write_bytes(lifecycle)
        pvc=self.obj("get","pvc",f"data-n{victim}","-n",self.namespace);self.save("pvc-before.json",pvc)
        # Retain the exact content-addressed SST bytes named by the selected cut.
        sst_pins=[]
        for number,f in enumerate(selected["files"]):
            raw=self.exec("minio","/bin/sh","-c",'export MC_HOST_local="http://${MINIO_ROOT_USER}:${MINIO_ROOT_PASSWORD}@127.0.0.1:9000"; mc cat "local/$1/$2"',"command",self.bucket,f["key"],cap=16*MIB,payload=True)[0]
            require(len(raw)==f["size"] and hashlib.sha256(raw).hexdigest()==f["sha256"],"selected remote SST bytes")
            (self.out/f"selected-sst-{number:03d}.bin").write_bytes(raw)
            sst_pins.append(dict(reference=f,retained=pin(self.out/f"selected-sst-{number:03d}.bin")))
        self.save("selected-sst-inventory.json",{"complete":True,"files":sst_pins})
        self.phase="podchaos-container-kill"
        self.apply({"apiVersion":"chaos-mesh.org/v1alpha1","kind":"PodChaos","metadata":{"name":"checkpoint-container-kill","namespace":self.namespace},"spec":{"action":"container-kill","mode":"one","containerNames":["kv9"],"selector":{"namespaces":[self.namespace],"pods":{self.namespace:[self.pods[victim]]}}}})
        fault=self.obj("get","podchaos","checkpoint-container-kill","-n",self.namespace);self.fault_uid=fault["metadata"]["uid"]
        self.k("wait","-n",self.namespace,"--for=condition=AllInjected","podchaos/checkpoint-container-kill","--timeout=30s")
        fault=self.obj("get","podchaos","checkpoint-container-kill","-n",self.namespace);self.save("injected-fault.json",fault)
        target=self.namespace+"/"+self.pods[victim]+"/kv9"
        require(any(x["type"]=="AllInjected"and x["status"]=="True"for x in fault["status"]["conditions"]),"actual AllInjected")
        require(any(x["id"]==target and x["phase"]=="Injected"and x["injectedCount"]>0 for x in fault["status"]["experiment"]["containerRecords"]),"actual selected container fault")
        def restarted():
            p=self.pod(victim);s=p["status"]["containerStatuses"][0]
            return p if s["restartCount"]>old["restartCount"]and "running"in s["state"] else None
        after=self.wait("samePod fresh container restart",restarted)
        self.wait("all voters Serving after checkpoint recovery",self.agreement)
        after=self.pod(victim);new=after["status"]["containerStatuses"][0];self.save("victim-after.json",after)
        require(before["metadata"]["uid"]==after["metadata"]["uid"] and before["spec"]["volumes"]==after["spec"]["volumes"],"same Pod and volumes")
        require(new["containerID"]!=old["containerID"] and new["lastState"]["terminated"]["containerID"]==old["containerID"] and new["lastState"]["terminated"]["exitCode"]==137,"actual old container death and fresh lifetime")
        require(self.exec(self.pods[victim],"cat","/data/kv9-store-lifecycle")[0]==lifecycle,"same durable store lifecycle")
        pvc_after=self.obj("get","pvc",f"data-n{victim}","-n",self.namespace);self.save("pvc-after.json",pvc_after)
        require(pvc_after["metadata"]["uid"]==pvc["metadata"]["uid"] and pvc_after["spec"]["volumeName"]==pvc["spec"]["volumeName"],"same PVC/PV")
        logs=self.k("logs","-n",self.namespace,self.pods[victim],"-c","kv9")[0];(self.out/"recovered-process.log").write_bytes(logs)
        matches=re.findall(rb'node (\d+) recovered checkpoint generation=(\d+) cut=(\d+) publication=(\d+)',logs)
        require(any(int(n)==victim and int(cut)==selected["index"] and int(pub)>int(cut) and int(gen)>0 for n,gen,cut,pub in matches),"new process's exact recovered checkpoint publication log")
        self.phase="recovered-read-write"
        self.check(b"keep",b"remote-value");self.check(b"deleted",None);self.check(b"overwrite",b"new-value")
        receipt=self.put(b"post-chaos",b"writable-after-remote-checkpoint")
        self.check(b"post-chaos",b"writable-after-remote-checkpoint")
        def caught_up():
            rows=self.statuses()
            return rows and set(rows)=={1,2,3} and all(int(r.get("driver_applied_index","0"))>=int(receipt["applied_index"])for r in rows.values()) and self.agreement(rows)
        self.wait("fresh acknowledged write caught up everywhere",caught_up)
        self.after_recovery()
        self.save("recovered-status.json",self.statuses())
        self.save("history.json",self.history)
        self.save("cell-acceptance.json",{"complete":True,"victim":victim,"namespace":self.namespace,"namespace_uid":self.ns_uid,"fault_uid":self.fault_uid,"selected_checkpoint":selected,"recovery_log_matches":[[int(y)for y in x]for x in matches],"old_container_id":old["containerID"],"new_container_id":new["containerID"],"old_exit_code":137,"same_pod_pvc_store":True,"fresh_write_receipt":receipt,"reclamation":pin(self.out/"reclamation.json"),"image_id":image_id,"tested_binary":binary})
        self.phase="scoped-cleanup"
        require(self.obj("get","namespace",self.namespace)["metadata"]["uid"]==self.ns_uid,"owned namespace cleanup identity")
        require(self.obj("get","podchaos","checkpoint-container-kill","-n",self.namespace)["metadata"]["uid"]==self.fault_uid,"owned fault cleanup identity")
        self.k("delete","podchaos","checkpoint-container-kill","-n",self.namespace,"--wait=true","--timeout=30s")
        self.k("scale","deployment","--all","-n",self.namespace,"--replicas=0")
        self.k("wait","--for=delete","pods","-l","app=kv9","-n",self.namespace,"--timeout=45s",seconds=55)
        # Reuse provision pods to retain a byte-exact stopped local data archive.
        for n in (1,2,3):
            manifest=json.loads((self.out/f"manifests/Pod-prepare-{n}.json").read_text())
            self.k("apply","-f","-",stdin=json.dumps(manifest).encode())
            self.k("wait","-n",self.namespace,"--for=condition=Ready",f"pod/prepare-{n}","--timeout=45s",seconds=55)
            raw=self.exec(f"prepare-{n}","tar","-C","/data","-cf","-",".",cap=128*MIB,payload=True)[0]
            archive=self.out/f"n{n}-stopped-data.tar";os.link(self.out/"commands"/f"{self.count:05d}"/"stdout",archive)
            members=[]
            with tarfile.open(archive,"r:")as tar:
                for entry in tar:
                    require(entry.isfile()or entry.isdir(),"ordinary stopped data archive")
                    if entry.isfile():
                        f=tar.extractfile(entry);members.append({"name":entry.name,"bytes":entry.size,"sha256":hashlib.file_digest(f,"sha256").hexdigest()})
            self.save(f"n{n}-stopped-data-inventory.json",{"archive":pin(archive),"files":members,"complete_readback":True})
        self.k("delete","namespace",self.namespace,"--wait=true","--timeout=60s",seconds=70)
        left=self.obj("get","namespaces")["items"];require(all(n["metadata"]["uid"]!=self.ns_uid for n in left),"owned namespace absent")
        self.save("protected-after.json",self.protection())
        before_protection=json.loads((self.out/"protected-before.json").read_text());after_protection=json.loads((self.out/"protected-after.json").read_text())
        require(before_protection["namespaces"]==after_protection["namespaces"] and before_protection["fault"]["spec"]==after_protection["fault"]["spec"],"historical protections unchanged")
        self.guard()
        result={"complete":True,"namespace":self.namespace,"namespace_uid":self.ns_uid,"owned_namespace_removed":True,"fault_uid":self.fault_uid,"actual_fault":"PodChaos/container-kill","acceptance":pin(self.out/"cell-acceptance.json"),"baseline_available_bytes":self.baseline,"minimum_available_bytes":self.minimum,"final_available_bytes":self.available(),"maximum_observed_decrease_bytes":self.storage.snapshot()["maximum_observed_total_decrease_bytes"],"maximum_added_budget_bytes":GIB,"floor_bytes":8*GIB,"filesystem_budget":self.storage.snapshot(),"protected_namespaces_unchanged":True,"old_fault_unchanged":True,"credentials_excluded":"credentials.json and Secret stdin omitted from portable evidence","scope":"One actual container death and remote-checkpoint restart on single-host Kind; no object-store power-loss durability, full21 or performance acceptance."}
        self.save("result.json",result)
        self.public_manifest()
        print(json.dumps(result,sort_keys=True),flush=True)


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary",type=Path,required=True)
    parser.add_argument("--binary-sha256",required=True)
    parser.add_argument("--inputs",type=Path,required=True)
    parser.add_argument("--output",type=Path,required=True)
    parser.add_argument("--image-preparation-failure",type=Path,help="Exact failed pre-fixture image attempt whose original budget and binary context remain authoritative")
    parser.add_argument("--retained-ca-base",action="store_true",help="Layer a newly pinned executable onto the exact retained CA-bearing image; no historical binary acceptance reuse")
    parser.add_argument("--ca-bundle",type=Path,default=Path("/etc/ssl/certs/ca-certificates.crt"))
    parser.add_argument("--prior-preparation",type=Path,help="Completed scoped cleanup of a failed pre-workload provision; reuse images and original space baseline")
    args=parser.parse_args();cell=Cell(args)
    def stop(signum,frame):raise InterruptedError("signal "+str(signum))
    signal.signal(signal.SIGTERM,stop);signal.signal(signal.SIGINT,stop)
    try:cell.run()
    except BaseException as error:
        cell.save("failure.json",{"complete":False,"phase":cell.phase,"error":str(error),"namespace":cell.namespace,"namespace_uid":cell.ns_uid,"fault_uid":cell.fault_uid,"available_bytes":cell.available(),"baseline_available_bytes":cell.baseline,"minimum_available_bytes":cell.minimum,"filesystem_budget":cell.storage.snapshot(),"owned_namespace_preserved_for_inspection":cell.ns_uid is not None,"historical_evidence_unchanged":True})
        if cell.history:cell.save("failed-history.json",cell.history)
        cell.public_manifest()
        raise


if __name__=="__main__":
    main()
