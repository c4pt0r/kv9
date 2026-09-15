# Bounded source and publication inventory

`scripts/build-workload.py` hashes every Git-listed tracked and untracked input
and retains tracked deletions in the same source identity map. Symlinks and
nonregular files remain rejected; the 10,000-file bound is unchanged.

Ordinary inputs retain **2 MiB per file / 64 MiB aggregate** limits. A separate
publication class permits **32 MiB per file / 256 MiB aggregate**, only under the
top-level `docs/` directory and only for `.md`, `.json`, `.jsonl`, `.csv`, `.tsv`,
`.txt`, `.log`, `.stdout`, `.stderr`, `.tar.gz`, or three-digit `.tar.gz.NNN`
archive parts. Code, manifests and unknown formats remain ordinary even inside
`docs/`; similarly named paths outside that directory receive no allowance.

This prospective accounting change accommodates approximately 167 MB of
published reports, inventories and archives measured on 2026-09-15. The 32 MiB
individual publication bound is retained from the prior archive policy. The
new 256 MiB aggregate leaves bounded publication headroom. Every accepted byte
is still hashed; archives are never decoded by the inventory, and historical
frozen helpers/results are unchanged. These limits are build-input accounting,
not benchmark resource limits or acceptance transfer.

Run the focused synthetic boundary/refusal controls and current-repository
positive snapshot with:

```sh
PYTHONDONTWRITEBYTECODE=1 taskset -c 6-15,22-31 python3 scripts/check-source-inventory.py
```

The current-repository check reports its actual source revision, dirty state,
input count and each class's byte total. A build still needs its own unchanged
before/after snapshots, source/proof qualification and explicit feature checks.
