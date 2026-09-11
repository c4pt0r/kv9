# Owned-buffer image-ready eleven-window preparation

Exact clean source `9be0c1963515ff974426faadef80482b847b13a5` uses the original `/tmp/kv9-owned-batch-release-first` output of `scripts/build-native-batch.py`: actual standalone correctness workload build followed by default server build. All 592 source files, source inventory, original manifest/binaries and both default opt3 Cargo graphs passed the unchanged source-binding verifier. No retrospective assembly or earlier server relabeling occurred.

The pending draft remains at `plan.pending.json`; `image-executed-plan.json` preserves the pre-image plan. Core runner, audit, contract, observers, history/checker/vendor/tool bytes and mandatory leaf reader remain inherited unchanged. The only helper adaptations are the verifier's expected revision and image-probe names/ownership labels. `preparation.diff` and `helper-lineage.json` retain the deltas.

Image-only session **53257**, PID **3284776**, exited **0**. The pinned-base build, actual executable payload/mode/utility/affinity probes, owned probe cleanup, Kind import and Docker/CRI binding passed once. The helper PID and exact probe container are absent. Fresh read-only checks verified both CRDs, three healthy Chaos Pods, tools/executable observer mount and library hashes; all eight historical namespace UIDs remain unchanged. The historical one-shot PodChaos still targets an absent old Pod. There is no NetworkChaos. No candidate namespace, node directory, database workload or fault has been created.

The final plan and input inventory are frozen for review. Actual runtime awaits root's completed process E2E acceptance and a fresh exact-plan one-use release. The proposed command, once released, is:

```sh
env PYTHONOPTIMIZE=0 PYTHONDONTWRITEBYTECODE=1 taskset -c 6-15,22-31 /usr/bin/python3 /tmp/kv9-owned-batch-recovery-preparation/chaos/run.py --plan /tmp/kv9-owned-batch-recovery-preparation/chaos/plan.json --release /tmp/kv9-owned-batch-recovery-preparation/chaos/launch-release.json --artifact /tmp/kv9-native-link-acceptance-9be0c19-attempt1
```

Preserve the original 11 windows, workloads/durations/capacities/deadlines, actual Service-VIP delay/30%-loss/partition and quorum effects, same-process exact-tuple reset, full histories including refused/unknown results, per-window positive/negative outcomes, fresh drains and exact owned cleanup. The reset remains explicitly non-Chaos SOCK_DESTROY. Preserve all failed attempts and original namespaces/UIDs; do not rerun to green.

After the sole runtime is terminal and scoped cleanup completes, use unchanged `audit.py` and `leaf_readback.py` with `--artifact` plus distinct fresh `--output`, and `verify_inputs.py --plan ... --output ...`. All three must pass. Require the same netem leaf's positive drop delta; never report parent-plus-child drops as distinct packets. Two serial fresh empty Serving/nonfatal/running-registry observations per voter after client exit, process/container lifetime exits, namespace UID absence and both owned node-directory absences remain mandatory.

This preparation is not new recovery acceptance. The eventual fixture supplies one-host volatile-tmpfs client-link/quorum correctness evidence, not throughput, cross-host or disk/power-loss evidence, full 21-window replacement, inter-voter partial loss/storage stalls or complete Rust/formal refinement.
