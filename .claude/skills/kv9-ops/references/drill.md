# The operations drill

`scripts/ops-drill.sh` rehearses every verb in `docs/OPERATIONS.md` on a local cluster and logs
each command with its output. Run it after changing replica plans, split, merge, drain, placement
or kv9-admin; read `docs/OPERATIONS-DRILL-2026-10-29.md` for the first run's record and findings.

## Running it

```bash
bash scripts/ops-drill.sh                          # ~4 min; prints PASS/ok/TIMEOUT lines as it goes
DRILL_KEEP=/mnt/data/kv9-work/drills bash scripts/ops-drill.sh    # copy the log + status files there
DRILL_DIR=/tmp/my-drill DRILL_BASE_PORT=28500 bash scripts/ops-drill.sh
```

Needs Docker (one MinIO container, `kv9-drill-minio`, port 27499), the three binaries (it builds
them), and ports 27501–27508. Run it on a fast disk: `/tmp` on NVMe, not the RAID (election churn,
see `diagnosis.md` §7). The run directory holds `drill.log`, `n<N>/status`, `n<N>.log`, `root.bin`.

## What it does, in order

0 MinIO · 1 three-voter formation · 2 first look (info, node list, health, status fields) ·
3 admit/join 4–7, promote 4 and 5, admit+revoke 8 · 4 group on 1,2,3 + keyspace `orders` + 400
rows · 5 metadata and group leader transfers (idempotent repeat) · 6 add-replica → 4, move 1 → 5,
stage watch · 7 refusals (app token, remove the leader) · 8 abort an add-replica · 9 split at
`r/00200`, refusal of a plan while the split is live, reads across the boundary · 9b merge back ·
10 drain node 2 `--wait`, stop it, health · 11 `kill -9` node 3, wait for repair and voter
replacement · 12 restart node 4 (the metadata leader) · 13 final state and row counts by kind.

Nodes run with `KV9_AUTO_REPAIR_REPLICA=1 KV9_AUTO_REPLACE_VOTER=1` and 20 s loss windows so
step 11 completes inside the drill; production defaults are 300 s.

## Reading the log

- `==================== [hh:mm:ss] N. <step>   (metadata term T)` — term climbing by more than
  the transfers and the leader restart means elections: check the disk.
- `[hh:mm:ss] $ <command>` then its output; `[exit=N]` only when non-zero. Exit 1 lines under
  step 7 and after health checks with a stopped node are expected.
- `[hh:mm:ss]   plans task 104: stage=…` — stage transitions with timestamps; the add's last line is
  `evidenced`, the move's `decided`.
- `ok: <wait> (Ns)` / `TIMEOUT after Ns: <wait>` — the waits that define pass/fail.
- `# diagnostics: …` — printed when a split is refused: plan stages and the task rows by kind.
- `DRILL_DONE` then `teardown`.

## Adding a scenario

Each step is `step "N. title"` followed by `adm …` (table or JSON), `admj …` (JSON for parsing),
`repl $'use orders\nget k'`, and waits: `wait_for "<label>" <secs> <predicate>`,
`watch_stages <group> plans|splits|merges <task> '<final regex>' <secs>`, `voters_are <group>
"[1, 2, 3]"`. `adm_retry` retries a verb five times (transfers); `adm_until_ok` waits until a verb
succeeds (right after a leader restart). `start_node N [ENV=…]`, `join_node N`, `kill ${pid[N]}`
manage processes; `sv N <field>` reads a status field. Keep the Add watch on `evidenced`, verify
`pid=` after a restart, and budget spare stores: placement never returns a group to a store that
held it, so a scenario that drains or repairs needs one never-hosting store per group per move.

Backup and restore are not in the drill script; their rehearsal is `scripts/backup-e2e.sh`
(about six minutes, MinIO): a two-range keyspace is backed up (one crash injected after `bind`),
half of it is deleted and overwritten, `restore create --as orders-restored --wait` brings the
backed-up rows back as a new keyspace, a second restore blocks `backup retire` and is aborted, and
the backup retires. Run it after changing capture, retention, backup or restore code.

Candidate scenarios not yet in the drill: a merge of groups whose members differ (refused), a
split under writes with the client retrying through the seal window, a drain of a three-voter
metadata group without a learner (waits and says so), a node whose disk fills
(`KV9_MIN_FREE_DISK_BYTES`), and auto-split (`KV9_AUTO_SPLIT_BYTES`) under load.
