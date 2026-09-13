# Vectored WAL comparison preparation evidence

Read the [comparison plan](../WRITE-SEGMENT-VECTORED-PERFORMANCE-PLAN.md) for the
complete workload, acceptance rules and unresolved capacity requirement.

`original-evidence.tar.gz` contains **149 original files / 1,828,502 decoded bytes**
in **483,229 compressed bytes**. SHA-256:

```text
af19b8d7eb2a1e39cb6e6f8cbbdc149e200db3c8b47fa4c436837d33ef3dd4c0
```

`inventory.json` maps each original absolute path to its size and SHA-256.
Archive names are `original/` followed by that path without its leading slash.
Extract only into a fresh review directory. Scripts retain their original host
paths; this is a review record, not a relocated runtime release.

The archive includes frozen preparation, ancestry, diffs, the second static
comparison against the frame harness, command arrays and all original control
outputs. Preparation inventory SHA-256:

```text
90efa1c6dadeffcfbd81e2518abbac629707cdaad2350b82fe3c679f06e3205c
```

All **8 + 15 + 5 = 28** controls pass at direct terminal `88a107/0`. Packaging
passes `2467b9/0`; independent archive readback passes `9112b1/0` and compares
every member with its original input. No executable, WAL or compressed workload
object is copied.

The historical capacity result and inventory are included. Their full original
metadata inputs are already published in the
[frame comparison evidence](../raft-frame-buffer-ab-plan-v1/README.md), whose
archive SHA-256 is
`b4e42830f908bf86f8244d60ff0f029d0632de6ec6851b29bc21620375e264c3`.
Those inputs are referenced rather than duplicated here.

The frozen preparation predates the controls and preserves that ordering; its
pending-control fields are superseded by the later successful terminal. Neither
this receipt nor the historical capacity scenario records a vectored smoke,
timed result, storage release or promotion. Historical failures remain retained
and labeled separately. No hosted CI was dispatched.
