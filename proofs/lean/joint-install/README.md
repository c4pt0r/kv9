# Joint generation installation model

`Installation.lean` proves a finite-trace projection of the offline destination
installer, not extracted Rust, full Raft, remote admission, or horizontal scaling.
The model has separate durable engine/protocol flags, a sealed generation, a
visible selector, a durable selector, a local receipt and a failed-owner flag.
It permits either selector outcome before directory sync and requires the new
selector after sync. The selected image and protocol are read from one generation.

Premises are exclusive parent/group ownership, immutable generations, correct
file/SST hash verification, successful sync durability and atomic rename on the
same local filesystem. The previous generation is complete and retained. Storage
corruption is refused; the model does not promise availability after corruption.
The concrete implementation tests both namespace outcomes; Chaos container kills
exercise process failure, not hardware power loss. Source authority, retention
owners, quorum safety, network snapshots, membership changes and startup are
outside this increment. No serving capability is returned by this API.

The source contract pins reviewed implementation and tests. SHA-256 pins detect
drift; they are not a refinement proof. The runner checks theorems, audits their
Lean dependencies and requires semantic defect controls to fail. It rejects
proof holes and custom axiom declarations. See docs/JOINT-SNAPSHOT-INSTALL.md for
measured implementation coverage and limits.

Run locally:

```sh
python3 scripts/prove-joint-install.py --lean /path/to/lean --output /outside/repo/new-directory
```
