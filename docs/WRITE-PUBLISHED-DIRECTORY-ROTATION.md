# Published-directory rotation supplement

On 2026-09-15 UTC, the first supplement established positive selected engine
WAL rotation on all three voters of candidate `483b8c3`, then failed during
Chaos Mesh target selection. The failed attempt is preserved. Leader-kill
recovery and rotation after recovery are still unverified by this supplement.
CRC main remains selected and there is no new performance result.

The exact default server/image and 16 MiB segment target are unchanged from
the [accepted full21 baseline](WRITE-PUBLISHED-DIRECTORY-CHAOS.md). A finite
single-worker burst uses 64 keys, 8,192-byte values and batches of 64. All
56 setup/traffic/verification calls succeeded. Traffic contains 45 BatchPut
and five BatchGet calls: 2,880 acknowledged write items / 23,592,960 input
value bytes. The complete atomic history passes the unchanged independent
reader. Initial and pre-fault drains each establish two fresh publications
on every voter; all three replicas reach applied/committed index 51.

Every store's checksum-valid selected topology names closed segment 1 and
active segment 2 at generation 2. Selected physical headers match stream,
sequence and predecessor, and topology bytes agree before/after header
capture. This provides positive rotation evidence beyond filenames or
offered-byte counts. It is a correctness workload, not a QPS measurement.

The fresh namespace omitted the installed controller's required annotation
`chaos-mesh.org/inject=enabled`. The controller reported that the namespace
was not enabled and selected no Pod. `Selected` and `AllInjected` remained
false, and all voter restart counts remained zero. The original 30-second
wait failed; no database crash or recovery was observed.

A separate corrected fixture adds that annotation during namespace creation,
reads it back immediately, and checks it again in the independent audit.
Its focused local control passes (`b1c398/0`). The existing 14 parser controls
are reused unchanged. Fault identity, AllInjected, exact container death,
same-PVC recovery, full value readback, subsequent rotation, fresh drains,
resource bounds and cleanup requirements are retained. The corrected runtime
has not yet executed at this checkpoint.

The failed run underwent a separate partial audit, complete reporting archive,
independent full-member/hash/EOF readback and exact owned cleanup. All four
remaining containers and their process trees exited, the namespace is absent,
and all eight historical namespace UIDs are unchanged. The partial reader
explicitly keeps `accepted=false` and the original runtime exit code 1.
[Portable original evidence and verifier](published-directory-rotation-prefix-v1/README.md)
retain the failure and these distinct successful preservation checks.

After complete corrected supplement acceptance, run the unchanged eight-smoke /
sixteen-timed matched write screen and report both throughput and latency.
The separately versioned storage v3 controls pass 71 local cases; actual net
capacity recovery and fresh full-campaign headroom remain execution gates.
No original industrial roadmap work package closes at this checkpoint.
