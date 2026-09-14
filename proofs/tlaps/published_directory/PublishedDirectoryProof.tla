--------------------- MODULE PublishedDirectoryProof ---------------------
EXTENDS Naturals, TLAPS

(* A token denotes a namespace edge, including its inode identity. A is the
   complete ancestor-edge set for one fixed canonical parent. D contains
   durable edges. e denotes the new segment's edge in that parent. A successful
   file fsync establishes F; successful parent fsync adds e to D. These are
   filesystem premises, not conclusions about the Linux implementation. *)
FullSync(D, A, e) == D \cup A \cup {e}
ParentSync(D, e) == D \cup {e}
Reachable(D, A, e, F) == A \subseteq D /\ e \in D /\ F

(* A capability may be issued only after full ancestor synchronization.
   Repeated rotations modify children, leaving these exact ancestor edges
   unchanged. This is the exclusive namespace-owner premise. *)
THEOREM InitialCapability ==
    ASSUME NEW D, NEW A, NEW e
    PROVE A \subseteq FullSync(D, A, e)
BY SMT DEF FullSync

THEOREM SuccessorEquivalent ==
    ASSUME NEW D, NEW A, NEW e, A \subseteq D
    PROVE ParentSync(D, e) = FullSync(D, A, e)
BY SMT DEF ParentSync, FullSync

THEOREM SuccessorReachable ==
    ASSUME NEW D, NEW A, NEW e, A \subseteq D
    PROVE Reachable(ParentSync(D, e), A, e, TRUE)
BY SMT DEF Reachable, ParentSync

THEOREM CapabilityPreserved ==
    ASSUME NEW D, NEW A, NEW e, A \subseteq D
    PROVE A \subseteq ParentSync(D, e)
BY SMT DEF ParentSync

THEOREM PriorEdgesPreserved ==
    ASSUME NEW D, NEW e
    PROVE D \subseteq ParentSync(D, e)
BY SMT DEF ParentSync

(* These two rules give an induction over any finite number of rotations;
   no bound on ancestor depth or number of namespace edges is imposed. *)
THEOREM ReachabilityPreserved ==
    ASSUME NEW D, NEW A, NEW e, NEW old,
           Reachable(D, A, old, TRUE)
    PROVE Reachable(ParentSync(D, e), A, old, TRUE)
BY SMT DEF Reachable, ParentSync

(* Reclaiming a child can remove its edge; that cannot invalidate the token
   when the removed edge is not one of the fixed ancestor edges. *)
THEOREM ChildReclaimPreservesCapability ==
    ASSUME NEW D, NEW A, NEW child, A \subseteq D, child \notin A
    PROVE A \subseteq (D \ {child})
BY SMT

(* Issuing a token after a crash requires synchronization again. This rule
   does not assume that an in-memory capability survives process failure. *)
THEOREM RecoveryReestablishesCapability ==
    ASSUME NEW D, NEW A, NEW e
    PROVE Reachable(FullSync(D, A, e), A, e, TRUE)
BY SMT DEF Reachable, FullSync

(* Explicit counterexamples show which premises cannot be removed. They do
   not model errors as successful fsync or conflate a file with its name. *)
THEOREM MissingCapabilityIsUnsafe ==
    ~Reachable(ParentSync({}, 2), {1}, 2, TRUE)
BY SMT DEF Reachable, ParentSync

THEOREM MissingParentSyncIsUnsafe ==
    ~Reachable({1}, {1}, 2, TRUE)
BY SMT DEF Reachable

THEOREM MissingFileSyncIsUnsafe ==
    ~Reachable(ParentSync({1}, 2), {1}, 2, FALSE)
BY SMT DEF Reachable, ParentSync

THEOREM ChangedAncestorIdentityIsUnsafe ==
    ~Reachable(ParentSync({1}, 2), {3}, 2, TRUE)
BY SMT DEF Reachable, ParentSync
=============================================================================
