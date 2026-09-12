------------------------ MODULE LeaderLeaseProof ------------------------
EXTENDS Integers, TLAPS

\* Conditional design lemmas, not a refinement of the current Rust server.
\* Integer-time specialization; the document separately proves real time.
\* Arbitrary integer scales may represent rational rates and durations.
\* a and b bound clock rates relative to real time. H is a real-time
\* exclusion horizon, D the leader duration, and E the voter duration.
\* Choose 0 < D <= a*H and b*H <= E; e.g. H=E/b, D<=E*a/b.
THEOREM LLStrictOrderChain ==
    \A x, y, z, w \in Int : x <= y /\ y < z /\ z <= w => x < w
BY SMT

THEOREM LLLeaderHorizon ==
    ASSUME NEW CONSTANT a \in Int, a > 0,
           NEW CONSTANT H \in Int,
           NEW CONSTANT D \in Int, D <= a * H,
           NEW CONSTANT elapsed \in Int, elapsed < D,
           NEW CONSTANT dt \in Int, a * dt <= elapsed
    PROVE dt < H
<1>1. a * dt < a * H BY LLStrictOrderChain, SMT
<1>2. dt >= H => a * dt >= a * H BY SMT
<1> QED BY <1>1, <1>2, SMT

THEOREM LLVoterPromiseUnexpired ==
    ASSUME NEW CONSTANT b \in Int, b > 0,
           NEW CONSTANT H \in Int,
           NEW CONSTANT E \in Int, b * H <= E,
           NEW CONSTANT dt \in Int, dt < H,
           NEW CONSTANT elapsed \in Int, elapsed <= b * dt
    PROVE elapsed < E
BY SMT

\* The send timestamp, not the response timestamp, starts the horizon.
THEOREM LLDelayedGrantContained ==
    ASSUME NEW CONSTANT s \in Int,
           NEW CONSTANT t \in Int,
           NEW CONSTANT g \in Int, s <= g, g <= t,
           NEW CONSTANT H \in Int, t - s < H
    PROVE t - g < H
BY SMT

\* A successful promise rejects votes already cast in a higher term,
\* as well as votes attempted during its protected interval. These are
\* separate premises: blocking only future votes is insufficient.
THEOREM LLNoHigherVoteBeforeRead ==
    ASSUME NEW CONSTANT g \in Int,
           NEW CONSTANT expiry \in Int,
           NEW CONSTANT t \in Int, g <= t, t < expiry,
           NEW CONSTANT higherVotes \in SUBSET Int,
           \A v \in higherVotes : v < g => FALSE,
           \A v \in higherVotes : g <= v /\ v < expiry => FALSE
    PROVE \A v \in higherVotes : v <= t => FALSE
BY SMT

\* Q can be any lease quorum and V any admissible election quorum.
\* Their intersection, rather than a hard-coded three-node majority,
\* is the required membership contract.
THEOREM LLQuorumExcludesHigherElection ==
    ASSUME NEW CONSTANT Nodes,
           NEW CONSTANT Q \in SUBSET Nodes,
           NEW CONSTANT V \in SUBSET Nodes, Q \cap V # {},
           NEW CONSTANT higherVotes \in [Nodes -> SUBSET Int],
           NEW CONSTANT t \in Int,
           \A q \in Q : \A v \in higherVotes[q] : v <= t => FALSE
    PROVE ~(\A q \in V : \E v \in higherVotes[q] : v <= t)
BY SMT

\* The captured commit frontier covers writes completed before invocation.
\* The exact immutable view contains the committed prefix through j.
\* Raft log safety, current-term commitment, and exact-view publication
\* establish these premises; a lease by itself does not establish them.
THEOREM LLReadCoversCompletedWrites ==
    ASSUME NEW CONSTANT Writes,
           NEW CONSTANT index \in [Writes -> Nat],
           NEW CONSTANT c \in Nat,
           NEW CONSTANT j \in Nat, c <= j,
           \A w \in Writes : index[w] <= c
    PROVE \A w \in Writes : index[w] <= j
BY SMT

\* A local validation after acquiring the immutable view rejects a pause
\* that crosses expiry, even if admission happened while the lease was live.
LLFinalGuard(sameGeneration, sameTerm, sameConfig, covered, now, end) ==
    sameGeneration /\ sameTerm /\ sameConfig /\ covered /\ now < end

THEOREM LLExpiredFinalCheckRejects ==
    ASSUME NEW CONSTANT now \in Int,
           NEW CONSTANT end \in Int, end <= now,
           NEW CONSTANT sameGeneration \in BOOLEAN,
           NEW CONSTANT sameTerm \in BOOLEAN,
           NEW CONSTANT sameConfig \in BOOLEAN,
           NEW CONSTANT covered \in BOOLEAN
    PROVE ~LLFinalGuard(sameGeneration, sameTerm, sameConfig, covered, now, end)
BY SMT DEF LLFinalGuard
=============================================================================
