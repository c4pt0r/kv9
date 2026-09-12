# Quorum trace production diagnostic build

The clean diagnostic source `1875e74141753cc6a55f025384b480b71ee84c1a`
built successfully in the first release transaction: session 29173, exit 0,
receipt c008fd. Root independently compared all 738 inventoried source files,
the retained executable SHA256 and the exact non-test optimized Cargo graph.

Server SHA256: `fe19660ed7182541c012f008a91eb532fc4e92f61be4103192dd53ae66b56762`.
Build manifest SHA256: `7a092f92546de9a6f1f2094c37346b29439917f27f83f52865de272b00392f93`.
The graph enables `quorum-trace` in kv9/server, `quorum-trace` and
`read-stage-timing` in raft, and no engine feature. No testing or alternate RPC
feature is enabled. The cache transaction invalidated first-party artifacts.
The existing 80 GiB build floor and 16 GiB additional reservation remain.

This reporting bundle contains metadata/logs, not executables. The builder also
retained a same-source helper workload; future comparisons keep the previously
pinned native benchmark client rather than silently replacing the timed client.
No runtime, trace capture, performance, recovery or Chaos acceptance follows
from this build. The selected uninstrumented runtime remains `11113f6`.
`inventory.json` binds every other reporting file by size and SHA256.
