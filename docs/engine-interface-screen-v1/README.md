# Engine interface diagnostic evidence

See the [report](../ENGINE-INTERFACE-SCREEN.md). All 90 processes / 152 rows
pass independent checks. Actual apply improves, but owned reads regress;
production remains unchanged and the previous selection gate remains failed.

[evidence.tar.gz](evidence.tar.gz) contains 648 members,
56,317,582 expanded bytes and 8,358,504 compressed bytes.
SHA-256: `52115914bcf5fe9584eb63505493c81d603dd6da4abbd16ed99a95cbf6926fd3`.
Every member size and hash passed archive readback. [members.json](members.json)
and [manifest.json](manifest.json) bind retained bytes and exclusions.

The packet contains both isolated source/build transactions, production engine
and common source copies, exact dependency graphs and changed dependency sources,
static call-boundary review, both independently validated preparations, all 88
measurement outputs and 90 launch/terminal records, and the full analysis.
`published-harness/` contains the runnable harness and orchestration tools.
No new production algorithm proof or Chaos acceptance is claimed.

Restore the original corpus from the [outlined packet](../outlined-mutation-path-v1/README.md),
member `outlined/groups.bin`, SHA-256
`d978ba9e49131f6277f975ecd333c33854845cc03bd14ee57a22fff915a08350`. Its qualified dependency/proof trees
supply the prior qualification inputs. Executables remain in the local retained
root with exact hashes in the manifest. Reproduction tools use recorded absolute
paths: restore those paths or adjust a fresh experiment before building, without
relabeling these measured artifacts.

Whole-pass p99 is the latency of 512 queries, not individual request p99.
All per-call/whole-pass, owned/resident and first-probe/warm results remain
separate in [analysis.json](analysis.json). The
[next isolated candidate plan](next-isolated-outline-plan.json) is prospective;
it has not been built, proved or timed in this packet.
