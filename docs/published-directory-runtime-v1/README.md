# Published-directory default release and recovery

Exact runtime: `483b8c3629b033734f5d7a2b8653a1352304a4b5`.
The archive retains 138 original members / 4,365,387 decoded bytes,
in 416,043 compressed bytes. Run `python3 verify.py` for full
member SHA256 verification without extraction. Original server/client binaries
remain local with exact paths, sizes and hashes in the inventory.

Default release 80235/57e676/0 and independent readback f68dc7/0 pass. Ordinary
recovery 39585/962723/0 and independent audit 027b71/0 pass: 364 complete operations,
333 OK and 31 unknown; 6 fresh exporter drains; 5 voter and 2 client lifetimes exited.
Source inventories, cache invalidation, compiler/codegen records, both complete
histories, original WAL bytes, audit outputs and scoped cleanup are retained.
These are same-host process recovery results, not a physical power-cut test.

The separate explicit v2 release/recovery phase policy uses 64 GiB plus 8 MiB
launch headroom, 48 GiB continuous floor, 16 GiB sampled decrease cap and the unchanged
1200-second deadline. Historical FNV helpers remain under `preparation/origin/`;
exact path/revision/policy differences remain under `preparation/diffs/`.
This policy does not replace the separately versioned benchmark or Chaos guards.
Inactive first-party dev-cache retirement reclaimed 5,172,379,648 observed bytes;
all 8 protected executable hashes remain unchanged. No source, dependency cache,
release cache, retained binary or original payload was removed.

The selected mainline remains CRC. Actual Chaos Mesh and matched database
throughput/latency acceptance of this candidate are still separate gates.
