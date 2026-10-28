# kv9-common

Foundational types shared by every kv9 crate: cluster/tenant/keyspace/region **ids**, the `Tenant` and
`Keyspace` model, `ApiType` (`Raw` is the only live variant; the retired transactional code is refused when
decoded), the multi-tenant **key codec** (mode byte `r`/`s` + 3-byte keyspace id + user key, with keyspace-id
width validation), `DataRange` (a region's range binding and epoch), the crate-wide `Error` (including the
typed refusals `StaleEpoch`, `RangeCrossesRegion`, `NotLeader`, `ReadUnconfirmed`, `WriteBackpressure`),
raw-TTL clock and expiry records, retention ownership, log budgets, store lifecycle markers and `Config`.

See `DESIGN.md` §3 (core concepts & data model), §3.4 (key encoding), §9.1 (the range-batch contract these
errors serve) and §12 (crate layout). This crate has no dependency on any other kv9 crate.
