# `kv9-admin`: one command-line tool for running a kv9 cluster

Design packet, 2026-10-28. Parent [#9](https://github.com/c4pt0r/kv9/issues/9). Builds on the
membership verbs ([VOTER-PROMOTION](VOTER-PROMOTION.md), [REPLICA-REMOVAL](REPLICA-REMOVAL.md),
[VOTER-REPLACEMENT](VOTER-REPLACEMENT.md)), the data-group protocol ([GROUP-CONTROL](GROUP-CONTROL.md),
[MIGRATION-AUTHORITY](MIGRATION-AUTHORITY.md), [SOURCE-CAPTURE](SOURCE-CAPTURE.md),
[DESTINATION-EVIDENCE](DESTINATION-EVIDENCE.md), [SOURCE-TRUNCATION](SOURCE-TRUNCATION.md)), the
status contract ([STATUS-PROCESS-IDENTITY](STATUS-PROCESS-IDENTITY.md)) and the Rust SDK
(`sdk/README.md`, `sdk/rust/kv9-client`).

> **Status: increments 1 and 2 implemented (2026-10-28).** `sdk/rust/kv9-admin` (binary `kv9-admin`,
> [README](../sdk/rust/kv9-admin/README.md)) on the Rust SDK's `admin` module; server side the
> `NodeStatus`, `ListNodes`, `ListDataGroups` and `ListTasks` RPCs, typed ids on the creation
> responses, and `KV9_ADMIN_PRINCIPALS`; `scripts/kv9-admin-e2e.sh` in the CI matrix. Implemented:
> contexts, leader resolution from one endpoint, `cluster info/status/health/endpoints`,
> `node list/describe/admit/revoke/promote/remove [--wait]`, `keyspace list/describe/create [--group]`,
> `group list/describe/create`, `task list/describe`, JSON output and the exit-code table.
> Deviations from the text below: `NodeStatus` also carries the node's local view of the directory
> (one follower address must be enough to find the leader, §3); `group create` derives its operation
> id from `--label` (or takes `--operation`, or mints one) because a group has no natural subject;
> JSON schemas are not yet frozen as files (§8); the `kv9 client` verbs still work as before and are
> not yet delegations (§9). **Increment 3 changed shape:** instead of a client-side driver for the
> thirteen migration verbs, the cluster drives a committed replica plan by itself
> ([REPLICA-ADDITION](REPLICA-ADDITION.md)); `kv9-admin group add-replica / move-replica` are the
> two verbs, and `group describe` shows the stage. `node transfer-leader` and `group
> transfer-leader` ([LEADER-TRANSFER](LEADER-TRANSFER.md)) move leadership in one bounded step.
> With `KV9_AUTO_REPAIR_REPLICA=1` a group leader commits the move plan for a lost voter by itself
> ([REPLICA-REPAIR](REPLICA-REPAIR.md)); `group describe` shows that plan like any other, and
> its live view covers the group's committed `members` (not only the creation's replicas).
> `node drain <id> [--wait]` ([DRAIN-NODE](DRAIN-NODE.md)) is the one-verb way to take a store out of
> service; `node list` shows `draining`/`drained`. `group split <task> --at <key>` splits a bound range and
> `group merge <low-task> <high-task>` merges two adjacent siblings back: the cluster seals, copies, verifies
> and publishes by itself ([RANGE-SPLIT](RANGE-SPLIT.md)); `group describe` lists `splits` and `merges` with
> their stage (`intended` → `populating` → `populated` → `published`).
> `node replace`, retention verbs and `group abort` remain open.

## The problem

Operating a kv9 cluster today means three different tools and a lot of shell:

| Need | What an operator does today |
| --- | --- |
| See the cluster | Read `<data-dir>/status` and `metrics.json` **on each node's disk** (`awk -F=` in every e2e script). Over the network only `ClusterInfo` (three counters), `ListKeyspaces`, `GetRegion`, `GetNodeEndpoint` exist. Role, term, voters, learners, applied position, pending admissions, data groups, replica pressure, checkpoint retention, budgets, `fatal=` — none of it is reachable remotely. |
| Change membership | `kv9 client admit-node / promote-node / remove-node / revoke-admission / change-node-endpoint`, each addressed **to the current leader by hand**; a follower answers `not_leader=true leader_node_id=N` and the operator looks up N's address themselves. |
| Create or move a data group | Up to **thirteen** `kv9 client` verbs for one migration (§6), half against the metadata leader and half against the group leader, carrying a 32-byte root digest copied out of a status file and a 16-byte operation id the operator must invent and keep. Every verb is idempotent and refuses divergence — the protocol is sound — but nothing drives it, resumes it, or tells the operator which step is next. |
| Script it | 25 hand-parsed `kv9 client` sub-verbs with per-verb flag sets and `key=value` stdout, no JSON, no stable exit codes beyond 0/1. |

The server side is in good shape: every administrative change is a committed, exact, idempotent task row
or configuration change, and every verb is confirmation-safe. What is missing is the **operator surface**:
one tool that finds the right leader, shows the cluster as the catalog sees it, drives multi-step
protocols from their committed state, and speaks JSON to scripts.

## 1. Principles

- **P1 — a pure gRPC client.** `kv9-admin` never opens a data directory, never links the server crate,
  and has no local authority. Everything it shows comes from a node over the wire; everything it changes
  goes through the same authenticated verbs an operator could call by hand. Provisioning that *must*
  touch a store (`store-prepare`, `root-create`, `init`, `join`, `install-migration-image`) stays in the
  `kv9` binary where the store lock and the bootstrap token live; `kv9-admin` prints those commands when
  a flow needs them (§6) but does not run them.
- **P2 — the catalog is the view.** Listings and descriptions come from committed catalog rows
  (`nodes`, `node_admissions`, `regions`, `region_peers`, `tasks`) read through the metadata leader, plus
  each node's own status document for what only that process knows (role, applied position, budgets).
  `kv9-admin` adds no cache, no state file and no second opinion: two operators running the same
  command see the same rows.
- **P3 — idempotent by construction, confirmed by output.** Every mutating command maps to one committed
  verb whose identity is derived, not invented: the tool computes operation ids the way the e2e
  scripts do and prints them. Re-running a command is always safe and reports `changed` or
  `confirmed`, exactly as the RPC does. A refusal is printed as the server's typed reason, never
  reinterpreted.
- **P4 — find the leader, never guess the state.** The tool resolves *which* leader a verb needs
  (metadata group or a data group) and follows `NotLeader` redirects through the node directory. It
  never infers that a change happened: `--wait` polls committed state until the effect is visible or a
  deadline passes.
- **P5 — a driver, not an authority.** Multi-step flows (`group migrate`, `node replace`) are driven by
  re-deriving the next step from committed rows each time (the same rule the runtime's own
  reconcile passes use). Killing the tool between steps loses nothing; `--resume` is the same command.
- **P6 — scripts first.** Every command has `--output json` with a versioned schema and a fixed exit
  code table. The e2e scripts that parse status files today are the first consumers.

## 2. Command tree

Nouns then verbs, in the shape of `etcdctl`/`tiup cluster`. Global flags come first.

```
kv9-admin [--context <name> | --endpoints <host:port,...>] [--token <t> | KV9_ADMIN_TOKEN]
          [--output table|json] [--timeout <dur>] [--yes] [--no-redirect] [-v]

  cluster   info                      counters, root digest, cluster id, metadata leader
            health                    verdicts (§7): quorum margin, lost voters, fatal lines, lag
            status  [--node <id>]     every node's status document (fan-out), or one node's
            endpoints                 node directory: id, address, generation, active

  node      list                      catalog nodes + admissions + raft role seen by the leader
            describe <id>             one node: catalog row, endpoint, status document, groups hosted
            admit    <id> --addr <a> [--ttl <dur>]        prints the one-time join ticket
            revoke   <id>
            promote  <id>            [--wait]
            remove   <id>            [--wait]            refuses below three voters (server rule)
            set-endpoint <id> --expect <addr>@<gen> --new <addr>
            replace  --lost <id> --with <id> [--wait]    promote-then-remove, I1 order (§5)

  keyspace  list
            describe <id|name>
            create   --name <n> [--tenant <id>] [--group <creation-task>]   raw on metadata, or on a data group

  group     list                      data groups: creation task, replicas (node, incarnation), activation, binding
            describe <creation-task>  rows + per-replica progress from the group leader
            create   --voters 1,2,3   derives the operation id, prints it and the creation task
            migrate  <creation-task> --to <node> [--resume] [--until <step>]   §6
            abort    <creation-task> [--operation <hex>]

  task      list     [--kind 103|104|105|106|...] [--after <id>] [--limit <n>]
            describe <id>

  retention apply    --file <request> ; owner <hex>
  version ; completion <shell>
```

Not in the tree on purpose: raw reads and writes (`kv9-repl` and the SDKs own the data plane; `kv9
client raw-*` stays for the quickstart), and any verb that bypasses the committed protocol (§10).

## 3. Resolving endpoints, leaders and identities

- **Contexts.** `~/.config/kv9/admin.toml` holds named clusters: endpoints, token env variable name
  (never the token itself), default timeout. `--endpoints` overrides; a single endpoint is enough — the
  node directory (`GetNodeEndpoint`, `cluster endpoints`) supplies the rest.
- **Metadata leader.** Ask any endpoint; on `NotLeader` the trailing metadata `kv9-leader-node-id`
  names the leader, `GetNodeEndpoint` maps it to an address, and the tool retries there (the SDK's
  redirect policy, bounded hops, `--no-redirect` to disable). The resolved leader is printed in `-v`.
- **Group leader.** For a data group the committed binding names the replicas; the tool asks each
  replica's status document for its role in that region, or uses `GetRegion` on a keyspace of the group.
- **Identities.** `root_digest` is read from the leader's status document (`cluster info` shows it);
  operation ids are derived as `sha16("kv9-admin/<verb>/v1" ++ root_digest ++ <subject>)` and printed,
  so a retried command confirms the same row (I3 of VOTER-REPLACEMENT applies). `--operation <hex>`
  overrides for a row created by hand.
- **Authentication.** One bearer token, the SDK's interceptor. `KV9_ADMIN_TOKEN` is read first, then
  `KV9_CLIENT_TOKEN`, so existing shells keep working.

## 4. What the server must add

Phase 1 needs three read-only RPCs and one authorization rule; the mutating verbs already exist.

| RPC | Served by | Returns |
| --- | --- | --- |
| `NodeStatus(NodeStatusRequest{})` | every node, about itself | the status document as a typed message: `pid, process_start_ticks, node_id, cluster_id, root_digest, store_incarnation, role, leader_id, term, conf_index, raft_committed, applied{term,index}, meta_voters, meta_learners, pending_admissions, bootstrap_state, fatal`, then the existing JSON lines (`voter_replacement`, `data_groups`, `replica_pressure`, budgets, retention) as `map<string, string> sections` so new lines need no proto change. Same renderer as the file: one function produces both, so file and RPC never disagree. |
| `ListNodes(ListNodesRequest{})` | metadata leader (linearizable read, like `GetNodeEndpoint`) | catalog `nodes` joined with `node_admissions` and endpoints: `node_id, address, generation, active, state, store_incarnation, admitted_until, raft_role_seen_by_leader` (voter / learner / none, from `peer_progress`), `matched, recent_active`. |
| `ListDataGroups` / `DescribeDataGroup` | metadata leader | committed creation (kind 100), activation and binding rows; replicas as `(node, store_incarnation)`; live migration (103) / evidence (104) / truncation (105) / removal (106) rows for the group; keyspaces bound to it. |
| `ListTasks(kind?, after, limit)` | metadata leader | the TASKS table page (TASK-HISTORY-PAGINATION already defines the cursor). |

**Authorization.** Today every authenticated principal may call every verb. `KV9_ADMIN_PRINCIPALS=
<name,...>` restricts the membership, data-group, retention and the new listing verbs to the named
principals; raw verbs are unchanged. Unset means "all principals" with a startup warning, so no existing
deployment breaks; the quickstart sets `admin` explicitly. (DESIGN §11 promised authorization from day
one; this is the first verb set that needs it.)

Nothing else changes server-side in phase 1. In particular the status file stays: scripts and the
chaos fixtures read it, and the RPC is the same renderer.

## 5. Membership flows

`node admit/revoke/promote/remove` are one RPC each, plus `--wait`, which polls `NodeStatus` of the
leader until `meta_voters`/`meta_learners` show the effect (the `MembershipChangeResponse` carries the
applied index; the wait is for the *configuration*, which is what the operator cares about).

`node replace --lost X --with R` is the manual form of VOTER-REPLACEMENT phase 1: it checks that R is a
caught-up learner (`ListNodes.matched` against `raft_committed`), promotes R, waits, then removes X,
and refuses at every step where the automatic driver would (live majority after the step, ≥ 3 voters,
never the leader itself). It prints the two committed configuration indices. If the automatic driver
is enabled and already mid-way, the tool observes that from `voter_replacement=` and only waits.

## 6. Driving a migration

The protocol is thirteen verbs against three parties; `group migrate <group> --to <node>` derives the
next one from committed rows and runs it, in this order (what each step is authority for is in the
linked packets, not restated here):

```
 step  party              verb                          committed fact that makes it "done"
  1    metadata leader    MigrateDataGroup              kind-103 intent row (operation, destination)
  2    group leader       AttachMigrationLearner        destination ∈ learners of the group
  3    group leader       PlanMigrationImage            manifest file (local to the tool)
  4    metadata leader    BindMigrationImage            image-owner binding in the retention ledger
  5    group leader       CaptureMigrationImage         capture record (cut term/index, objects uploaded)
  —    destination host   kv9 install-migration-image   OFFLINE: store stopped. The tool stops here,
                                                        prints the exact command and the record file path,
                                                        and exits 5 ("needs operator step").
  6    destination        EmitInstallEvidence           receipt file
  7    metadata leader    RecordInstallEvidence         kind-104 evidence row
  8    metadata leader    RecordSourceTruncation        kind-105 row (floor term/index from the capture)
  9    group leader       TruncateSourceLog             source floor applied
 10    group leader       PromoteMigrationVoter         destination ∈ voters
 11    metadata leader    RecordSourceRemoval           kind-106 decision row (source node, incarnation)
 12    group leader       RemoveSourceReplica           source ∉ voters, |voters| unchanged
```

`--resume` is the same command: every step first asks whether its fact already holds (the verbs
themselves answer `confirmed`), so the tool can be killed anywhere. `--until <step>` stops early for the
e2e scripts that test one refusal at a time. `group abort` runs `RecordMigrationAbort` then
`DetachAbortedLearner`, the committed abort path of MIGRATION-ABORT. Files the protocol exchanges
(manifest, capture record, receipt) live in `--workdir` (default `./kv9-admin-work/<operation>/`) and
their digests are printed, because the install step on the destination host needs the record file.

## 7. `cluster health`

Verdicts, each with the rows that justify it, exit 0 only when all are `ok`:

| verdict | rule |
| --- | --- |
| `quorum_margin` | metadata voters minus lost voters (from the leader's `voter_replacement.voters[].state`) minus majority; `warn` at 0, `fail` below. |
| `voter_lost` | any voter `lost` or `suspected`. |
| `fatal` | any node whose status has a non-empty `fatal=`. |
| `unreachable` | a directory node that answered neither `NodeStatus` nor a dial within the timeout. |
| `apply_lag` | `raft_committed - applied_index` above a threshold on any node (metrics `apply_lag`). |
| `admissions_expiring` | pending admissions whose TTL ends within an hour and that have not joined. |
| `group_control_error` | `data_group_control_error=` non-empty anywhere. |
| `migration_stalled` | a kind-103 intent with no newer row for the group for longer than `--stall`. |

The same table is what `cluster status --output json` carries; `health` only evaluates it.

## 8. Output, exit codes, errors

- `table` on a terminal, `json` otherwise or with `--output json`. JSON documents carry
  `{"schema_version": 1, "command": "...", "resolved_leader": {...}, "result": ...}`; the result shape
  per command is frozen in `sdk/rust/kv9-admin/schema/*.json` and checked by a controls script.
- Exit codes: `0` ok · `1` refused by the server with a typed reason (not leader after redirects,
  admission refused, precondition, fewer-than-three-voters, divergence) · `2` usage · `3` transport
  (dial, deadline, unauthenticated) · `4` partial (fan-out with some nodes unreachable; the JSON says
  which) · `5` needs an operator step (§6) · `6` `--wait` deadline passed with the change proposed but
  not yet visible.
- Mutating verbs print the plan (verb, target leader, derived identity) before acting and ask for
  confirmation on a terminal; `--yes` skips it. `node remove` and `group migrate` step 12 require the
  node id typed back unless `--yes`.

## 9. Where it lives and what moves

- New workspace member `sdk/rust/kv9-admin` (binary `kv9-admin`), on `kv9-client` with a new
  feature `admin` that exposes the admin RPC stubs and typed refusals already compiled from the same
  proto. One proto compile, one `classify()`, one redirect policy. No dependency on `kv9-server`.
- `kv9 client` keeps `create-keyspace` and `raw-*` (the quickstart path). The other 23 verbs become
  one-line delegations that print `use: kv9-admin <noun> <verb> ...` to stderr and keep working for
  one release while the Python e2e scripts move their `command('client', ...)` helpers over; then they
  are removed, together with `scripts/local_admin_client.py` (its routing job is P4).
- Controls: `scripts/check-membership-controls.py` and friends pin server-side tests, not CLI strings,
  so they survive; a new `scripts/check-admin-controls.py` pins the JSON schemas and the exit-code
  table.

**Increment 1 (next):** crate skeleton, contexts, leader resolution, `cluster info/endpoints`,
`keyspace list/describe/create`, `node list/describe/admit/revoke/promote/remove [--wait]` and
`cluster status/health` on the new `NodeStatus` + `ListNodes` RPCs with `KV9_ADMIN_PRINCIPALS`;
`scripts/voter-replacement-e2e.sh` ported from status-file `awk` to `kv9-admin --output json` as the
first consumer. **Increment 2:** `group list/describe/create`, `keyspace create --group`, `task list`,
`ListDataGroups/ListTasks`. **Increment 3:** `group migrate/abort` driver, `node replace`, retention
verbs, shell completion, removal of the delegated `kv9 client` verbs.

## 10. Evidence this packet must produce

- Unit tests for identity derivation (same inputs → same operation id, printed hex matches the
  server's confirmation), redirect resolution against a fake directory, and the health rules on
  recorded status documents.
- `scripts/kv9-admin-e2e.sh`: three voters + two learners; `node admit/promote/remove --wait`
  against a follower endpoint (redirect), `cluster health` goes `ok → warn → ok` across a voter kill
  and automatic replacement, every command run twice shows `confirmed` the second time, JSON validates
  against the schemas, exit codes match the table. In the CI e2e matrix.
- The migration e2e scripts (`removal-e2e.py`, `source-truncation-e2e.py`, `migration-abort-e2e.py`)
  pass unchanged through the delegated verbs, then once more through `group migrate --until <step>`.
- `docs/QUICKSTART.md` and `README.md` show `kv9-admin` for everything that is not provisioning or
  the raw quickstart.

## 11. Out of scope, on purpose

- **Provisioning and offline store surgery** (`store-prepare`, `root-create`, `init`, `join`,
  `install-migration-image`, quorum-loss recovery): local, token-gated, store-locked; stays in `kv9`.
- **Placement decisions** (which node should host what, how many standbys): D06. `kv9-admin` shows and
  executes, it does not choose.
- **A long-running daemon, a web UI, or a metrics backend.** `metrics.json` is already a fixed-inventory
  export for scrapers; `cluster status` exposes the same numbers once over gRPC.
- **Multi-cluster orchestration and rolling upgrades.** Contexts name clusters; nothing coordinates
  across them.
