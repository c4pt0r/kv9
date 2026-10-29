---
tags: [kv9-ops]
max_turns: 20
timeout_seconds: 600
allowed_tools: [Read, Glob, Grep, Skill, Bash]
---

I ran `kv9-admin node drain 2 --wait --wait-secs 600` on our kv9 cluster after we had split and then merged the `orders` range earlier today. It came back with exit 6 "node 2 not drained after 600 s". On the metadata leader the status file says:

node_drain={"parallel":1,"this_store_draining":false,"drains":[{"task":130,"node":2,"incarnation":"e27a…","memberships_left":[100,101,102,103],"plans_unsettled":[132],"meta_role":"voter","finished":false,"waiting":"1 move plan(s) in flight"}],"in_flight":null,"events":[{"at_unix":1793306277,"node":2,"step":"move_plan","outcome":"region 100: node 2 → node 6: planned plan 132 (migration 131)"}],"error":null}

`group list` shows region 100 with a sealed keyspace row and splits 117 published, regions 101/102 sealed with merge 125 published, and region 103 live. What is going on and what should I do? Don't run anything, just tell me.
