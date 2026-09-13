# Full CRC regression tool evidence

Read the [qualification report](../WRITE-CRC-FULL-REGRESSION-TOOLS.md).

`original-evidence.tar.gz` contains **186 original files / 2,258,294 decoded
bytes**, stored in **563,357 bytes**, SHA-256:

```text
a36c3ef4979e042d27925bbf91e36d5e996cfaa4a0f9a875f071d6147b464af7
```

`inventory.json` maps each original absolute path to its size and hash. Members
use `original/` followed by the path without its leading slash. Extract into a
fresh review directory; runtime scripts still bind original host paths.

- Runtime preparation: inventory
  `392d842332f324d8689b8bf09c7aa46a4f058495d943f91ba3dfd1e25caf15dd`;
  full protocol `b849b46085787a7a4512b771fd3d9bd967722b25cd0149f1d4cbdfe234445fd8`.
- Runtime-tool control terminal `a1698d/0`: **8 + 15 + 5 + 7 = 35 tests pass**.
- Reporting preparation: inventory
  `5a8442e35eb3237cda2d64c74d98a674e48319215bfab8c4bec007cb174297a6`;
  unchanged arithmetic core
  `827110ec99b2ae82f042e40fc81844263eefe4c484680ef6d8ab54653515b17f`.
- Reporting control terminal `233089/0`: **13 tests pass**.

The archive retains both preparations, their original sources/diffs and prior
correction lineage, all five command outputs, supervisors and terminal records.
Its frozen pending-control fields predate those terminals. No smoke, timed
workload, benchmark audit or real performance report has run. Capacity readiness
remains false. Independent archive readback is recorded in `readback.json`.
