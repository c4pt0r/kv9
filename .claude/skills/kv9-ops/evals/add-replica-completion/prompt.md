---
tags: [kv9-ops]
max_turns: 20
timeout_seconds: 600
allowed_tools: [Read, Glob, Grep, Skill, Bash]
---

quick q about kv9: I added a replica to data group 100 on node 4 about two minutes ago with `kv9-admin group add-replica 100 --to 4`. `group describe 100` shows the plan at stage "evidenced" and it has been sitting there since; the leader view says voters [1, 2, 3, 4]. Is it stuck? Should I abort it and retry, or wait for "decided"? Also, if I later want to split this group, anything I need to know given it now has four voters?
