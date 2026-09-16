# WAL preallocation Chaos evidence

Read the [report](../WAL-PREALLOCATION-CHAOS.md) and [result](result.json).
Actual runtime is `47781/38e7ef/0`; all six post phases are `55667/4cbed9/0`.
The [independent audit](audit.json), [cleanup](cleanup.json) and
[all-lifetime readback](all-lifetimes.json) have separate scopes and timestamps.
The pre-cleanup audit's `cleanup_complete=false` is preserved, not rewritten.

`metadata.tar.gz` contains **2,495 original members / 38,580,640 logical bytes**,
compressed to **4,029,620 bytes**. SHA-256:
`7518653c9ac17c4fbcb2aa98dbaf1336638b8200fe7bb8cb3b70f5c1ef786f2d`.
Every member is checked against `metadata-inventory.json`; the entire portable
selection passed independent member readback (`61178/0c0aa8/0`). The three
complete history hashes from the independent audit are present in the archive.

Paths within the archive preserve their original absolute-path suffixes under
`mnt/data/kv9-work/`. Members include current helper source, build manifests,
fresh unit-control results, full client histories, selected window observations,
drains, command logs and the separate post-cleanup records. The archive's
embedded `inventory.json` identifies the full original local archive contents.

`excluded-original-members.json` explicitly identifies omitted executables,
duplicate copies, large observer/pressure streams and large intermediate
command snapshots. Those originals remain in the separately verified
[full local archive](full-archive-result.json), at
`/mnt/data/kv9-work/wal-preallocation-chaos-preparation-20260915-first/archive-first/evidence.tar.gz`.
Its 4,836 members / 89,254,172 compressed bytes were all read back before
cleanup. The portable packet cannot replay checks requiring omitted originals.
No private kubeconfig is included.

The initial portable selector stopped on the archive's embedded inventory,
which is not recursively listed as its own member. That failed output remains
local; `publication-failure.json` and `package-first.py` preserve the cause.
`package.py` uses a fresh output directory and checks the embedded inventory
against its separately retained digest. Actual runtime/post acceptance preceded
this publication-only failure and was not rerun.

Original preparation and runtime outputs remain under
`/mnt/data/kv9-work/wal-preallocation-chaos-preparation-20260915-first` and
`/mnt/data/kv9-work/kv9-chaos-e2e.Bgl9Cx`. This packet establishes actual local
Chaos acceptance, not database throughput, Redis parity or default promotion.
