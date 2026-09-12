# Explicit Linux lease clock and sampled-time bounds

Tracking: [#9](https://github.com/c4pt0r/kv9/issues/9), #20.

The experimental `LinuxBoottimeClock` supplies actual Linux `CLOCK_BOOTTIME`
samples to the existing lease adapter. Installation requires an explicit
`ClockBounds` declaration and compatible durable `LeasePolicy`. Default server
startup remains Safe ReadIndex. This implementation does not establish physical
clock-rate bounds, enable a CLI switch or select the lease performance path.

## Platform contract

Let the ideal local clock `C` advance at rates in `[a,b]`, where
`a=1-rho`, `b=1+rho` and `rho=drift_ppb/1e9`. A returned sample at its observation
time differs from `C` by at most `epsilon=error_ns`, in either direction.
Consequently a difference of two samples has error at most `2*epsilon`.
Offsets between different machines need not agree; timestamps do not cross
the wire. The bounds must include quantization, kernel clock discipline, CPU
migration and every supported scheduling, host-suspend or virtualization state.

Linux documents that
[`CLOCK_BOOTTIME`](https://man7.org/linux/man-pages/man3/clock_gettime.3.html)
includes suspended time, whereas `CLOCK_MONOTONIC` and `CLOCK_MONOTONIC_RAW`
exclude it. The adapter therefore requests BOOTTIME directly, with no fallback
to either monotonic clock or wall time. `clock_getres` is checked against the
declared sample error at construction; reported resolution alone does not prove
accuracy or a real-time rate bound. Linux does not promise a universal numeric
bound for every machine. In particular,
[virtualized timekeeping](https://docs.kernel.org/virt/kvm/x86/timekeeping.html)
requires platform-specific scrutiny. A stopped or rolled-back VM clock can
violate the contract even when an ordinary process-stop test passes.

Deployments must establish conservative bounds for their supported machines and
operating states before enabling leases. Supplying numbers is an explicit caller
assertion, not an automatic qualification. Until then use default Safe ReadIndex.
Actual host suspend, migration, independent clock calibration and lease-enabled
Chaos Mesh acceptance remain separate required evidence; no current test infers
an oscillator bound from two clocks derived from the same hardware.

## Sampling-error containment and recovery proof

Let `E` be the voter promise, `D` the leader duration, and `R` the restart
quarantine, all in sampled-clock nanoseconds. A leader renewal starts at real
time `s`; each voter samples its grant at `g >= s`. At a successful final read
sample time `t`, the elapsed leader sample is less than `D`, so

```text
a*(t-s) - 2*epsilon < D
t-s < (D + 2*epsilon)/a.
```

The corresponding voter elapsed sample is at most
`b*(t-g)+2*epsilon <= b*(t-s)+2*epsilon`. Thus the sufficient condition

```text
(D + 2*epsilon)*b <= (E - 2*epsilon)*a
```

makes that sample strictly less than `E`. No competing vote is allowed. This
substitutes sampled elapsed-time bounds into the existing quorum-intersection
and exact-view [linearizability proof](LEADER-LEASE-PROOF.md).

On recovery, an old promise's remaining real duration is at most
`(E + 2*epsilon)/a`. A new clock that has advanced by sampled duration `R`
has waited at least `(R - 2*epsilon)/b` in real time. Hence require

```text
(R - 2*epsilon)*a >= (E + 2*epsilon)*b.
```

The existing integer timing controller computes
`D=floor(E*a/b)-M` and `R=ceil(E*b/a)+M`. Leader containment needs
`M >= 2*epsilon*(1+a/b)`; recovery needs the larger
`M >= 2*epsilon*(1+b/a)`. Since `a+b=2`, one shared safe integer margin is

```text
M >= ceil(4*epsilon*1_000_000_000 / (1_000_000_000 - drift_ppb)).
```

`ClockBounds::minimum_margin_ns` implements that expression with checked `u128`
intermediates and checked conversion to `u64`. Its largest intermediate is
`4*u64::MAX*1e9`, below `u128::MAX`; invalid drift, an unrepresentable margin,
zero usable leader duration and recovery overflow are refused. A policy may
declare more drift than the clock; the calculation uses that policy's larger
bound. Simply reserving the leader-only margin is unsafe for recovery.

Three nonlinear real-arithmetic SMT queries separately establish containment,
recovery and the common-margin implication. Removing the sampled-error reserve
from either timing bound or replacing the recovery margin by the leader margin
produces a satisfiable countermodel. These extend the clock premises of the
existing TLA+/TLAPS authority proof; they do not machine-check the Linux kernel,
Rust refinement or physical oscillator.

## Source binding and failure behavior

`LeaseClock::validate_policy` is called under installation before writing the
durable lease epoch. The concrete clock rechecks its stored bounds there, so a
clock created against a conservative policy cannot subsequently be installed
against an insufficient margin or smaller drift declaration. Custom clock
implementations retain the explicit trusted-clock contract; the default trait
method does not qualify them.

Each clock object gets a nonzero, non-reused process-local domain. Exhaustion
refuses creation. These domains never replace the durable peer incarnation or
recovery quarantine. Reopening a lease voter still needs the durable opt-in and
a fresh quarantine, including when the feature is absent from the new binary.

Sampling performs a fresh `clock_gettime(CLOCK_BOOTTIME)` through libc. It has
no application mutex, filesystem access, cached timestamp or explicit retry
loop. The libc/vDSO/kernel operation may be preempted. The syscall's return code,
timespec shape and nanosecond conversion are checked; any sampling failure
permanently fences the clock object. The installed peer also fences regression,
domain changes and sampling failure. Returning to apparently valid samples
cannot resurrect that installation's voting or read authority.

The final clock check follows acquisition of the exact retained read view.
Preemption inside the clock call can move its observation time within that call;
the view already exists, and the observation remains inside the RPC lifetime.
A delayed response consumes the retained view. It cannot authorize a new read
or replace the snapshot after expiration. A process pause before observation
must advance the qualified clock sufficiently to expire the lease; a paused
clock that violates the declared bounds cannot be repaired by rereading it.

## Validation scope

[Retained local evidence](lease-clock-v1/README.md) records 593 passing library
tests, the default restart check, the actual process-pause integration,
default/experimental compilation and Clippy. The three SMT queries and three
countermodels pass. No clock-platform or lease-performance qualification is
inferred from this checkpoint.

The dedicated integration test uses a separate owned child process and observes
its actual `/proc` stopped state after `SIGSTOP`. It resumes that child with
`SIGCONT` after 300 ms. The child obtains a real clock-backed controller lease
from a single voter's actual promise after quarantine, verifies a successful
pre-pause read, then checks that both retained and newly requested read authority
expire. The parent checks the child's clock advancement and terminal success.
All waits have deadlines and a guard kills/reaps the owned child on failure.
This exercises actual process scheduling suspension and the clock/controller
boundary; it does not constitute host suspend, three-node RPC, Chaos Mesh or
physical clock calibration.

Run `scripts/check-lease-clock-proof.py --z3 <pinned-z3> --output <new-path>`
for the three proof queries and their three countermodels. Source validation
also includes margin/overflow and sticky-failure tests, rejection before durable
installation, the existing default/experimental suites, and the feature-disabled
restart check. Results and remaining qualification steps are recorded separately;
no new throughput or latency result follows from these checks.
