# kv9-admin

Cluster administration for kv9, built on the [Rust SDK](../kv9-client/). A pure gRPC client: it
opens no data directory and keeps no state. Design and the full command tree: [docs/KV9-ADMIN.md](../../../docs/KV9-ADMIN.md);
this is increment 1 of it.

```bash
cargo build -p kv9-admin
export KV9_ADMIN_TOKEN=<one of the server's KV9_CLIENT_TOKENS>
kv9-admin --endpoints 127.0.0.1:20161 cluster info          # one node's address is enough
kv9-admin --endpoints 127.0.0.1:20161 node list
kv9-admin --endpoints 127.0.0.1:20161 cluster health
```

```text
kv9-admin [--context <name> | --endpoints <[id=]host:port,...>] [--token <t>]
          [--output table|json] [--timeout <secs>] [--yes] [--no-redirect] [-v]

  cluster  info                      counters, cluster id, root digest, metadata leader
           status [--node <id>]      every node's status document (fan-out), or one node's
           health                    verdicts: quorum margin, lost voters, fatal, unreachable, lag, ...
           endpoints                 the node directory: id, address, generation, active
  node     list                      directory + admissions + the leader's Raft view of each peer
           describe <id>             one node: directory row and status document
           admit <id> --addr <a> [--ttl <secs>]       prints the one-time join ticket
           revoke <id>
           promote <id> [--wait [--wait-secs <n>]]   learner → voter
           remove <id>  [--wait [--wait-secs <n>]]   refused below three voters
           transfer-leader --to <id> [--wait]        hand metadata leadership to a caught-up voter
  keyspace list | describe <id|name> | create --name <n> [--tenant <id>] [--group <creation-task>]
  group    list                      committed data groups: task, region, replicas, keyspaces, migrations
           describe <creation-task>  the rows plus every replica's live view (state, role, term, applied)
           create --voters <id,id,id> [--operation <hex> | --label <text>]
           add-replica <creation-task> --to <node> [--operation <hex> | --label <text>]
           move-replica <creation-task> --from <node> --to <node> [--operation <hex> | --label <text>]
           transfer-leader <creation-task> --to <node>   hand the group's leadership to a caught-up voter
  task     list [--kind <n>] [--after <id>] [--limit <n>]   the committed TASKS history, paged
           describe <id>
  version
```

**Identities.** `group create` needs a 16-byte operation id, the group's immutable retry identity:
`--operation <hex>` passes one, `--label <text>` derives one from the certified root digest and the
label (so repeating the command confirms the same group), and with neither the tool mints a random
one and prints it. The root digest itself is read from the leader's status document.

**Resolution.** Every node answers `NodeStatus` about itself, including its local view of the node
directory, so the endpoints given become a peers map and name the metadata leader; directory reads
and membership verbs then go to the leader and follow `NotLeader` hints (`--no-redirect` disables
that). `cluster status` and `health` fan out to every address the directory lists.

**Contexts.** `~/.config/kv9/admin.toml` (or `KV9_ADMIN_CONFIG`) names clusters; `--context` or
`KV9_ADMIN_CONTEXT` selects one, `default = "..."` applies when neither is given:

```toml
default = "local"
[context.local]
endpoints = ["1=127.0.0.1:20161", "2=127.0.0.1:20162", "3=127.0.0.1:20163"]
token_env = "KV9_ADMIN_TOKEN"
timeout_secs = 5
```

The token comes from `--token`, `KV9_ADMIN_TOKEN`, `KV9_CLIENT_TOKEN`, or the context's `token_env`.

**Output and exit codes.** A table on a terminal, JSON otherwise or with `--output json`:
`{"schema_version": 1, "command": ..., "resolved_leader": {...}, "result": ..., "partial": [...]}`.
Exit `0` ok · `1` refused by the server with a typed reason (not an administrator, fewer than three
voters would remain, not found, …) or `cluster health` not `ok` · `2` usage · `3` transport (dial,
deadline, bad token) · `4` partial (some nodes unreachable; JSON says which) · `6` `--wait` deadline
passed with the change proposed but not yet visible in the configuration.

**Mutating verbs** print their plan and ask on a terminal (`node remove` wants the node id typed
back); `--yes` or a non-terminal stdin skips the question. Every verb is idempotent on the server:
repeating one confirms the same configuration.

**Who may call it.** `KV9_ADMIN_PRINCIPALS=admin,ops` on the servers restricts the administrative
RPCs to those principals (by the name in `KV9_CLIENT_TOKENS`); other tokens keep the data plane and
get a typed `PermissionDenied`. Unset means every authenticated principal, as before.

**Replica plans.** `group add-replica` and `group move-replica` commit one plan; from there the
cluster attaches the learner, captures and pins the image, installs it inside the destination
process, records the evidence, promotes the voter and (for a move) retires the source, with no
further operator action ([REPLICA-ADDITION](../../../docs/REPLICA-ADDITION.md)). `group describe`
shows the plan's catalog stage (`planned → captured → pinned → evidenced → decided`) and every
replica's live view, including the voter set; each node's status line `replica_plans=` lists the
steps it took.

`scripts/kv9-admin-e2e.sh` runs the administrative verbs against a fresh five-node cluster;
`scripts/replica-addition-e2e.sh` runs an addition and a move on a real MinIO container.
