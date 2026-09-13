# Frame-buffer comparison preparation evidence

Read the [comparison plan](../WRITE-RAFT-FRAME-BUFFER-PERFORMANCE-PLAN.md) for the
workload, acceptance rules and unresolved capacity requirement.

`original-evidence.tar.gz` contains **164 original files / 6,631,706 decoded bytes**
in **958,948 compressed bytes**. SHA-256:

```text
b4e42830f908bf86f8244d60ff0f029d0632de6ec6851b29bc21620375e264c3
```

`inventory.json` maps every original absolute path to its size and SHA-256. Each
archive member has the relative name `original/` followed by that path without
its leading slash. Extract only into a fresh review directory. Scripts retain
their original host paths; the archive is a review record, not a relocated
runtime release.

The archive includes:

- Frozen A/B preparation, ancestry, diffs and command arrays. Preparation
  inventory SHA: `d949a45598b4a15e3db387c02d8d99a4262efe962f78306e0768e656bcd80a98`.
- Original local control supervisor, result, outputs and terminal. All **8 + 15
  + 5 = 28** controls pass at direct terminal `b424dc/0`. Result SHA:
  `82f88eeb5d13092b1a8d7142434fa6283e6a5650fff1c37e2230e36ba4bcb590`.
- Capacity reproducer, result and all eleven bound metadata inputs from the
  accepted CRC comparison. Capacity inventory SHA:
  `93831f17cb6c7e4df40f724d83eb0782836a6420b8917fe8efe4e022d1316780`;
  `result-first.json` SHA:
  `dcfdcf77adbb8fc4ebea71f5baea5b01b3c239bcec1fa43778eef6efe16da047`.
- Publication script; no executable, WAL or compressed workload object is copied.

The frozen preparation predates execution of the controls and preserves that
ordering. Its pending-control fields are superseded by the later successful
terminal. Neither this receipt nor the capacity analysis records a frame smoke,
timed result, storage release or default promotion. Historical import failures
remain retained. Independent archive readback is recorded in `readback.json`.
