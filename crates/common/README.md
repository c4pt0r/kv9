# kv9-common

Foundational types shared by every kv9 crate: cluster/tenant/keyspace/region/txn-group
**ids**, the `Tenant` and `Keyspace` model, `ApiType` (`Txn`/`Raw`), the multi-tenant
**key codec** (mode byte + 3-byte keyspace id + user key, with keyspace-id width
validation), the `TimeStamp`/`Hlc` clock types, the crate-wide `Error`, and `Config`.

See `DESIGN.md` §3 (core concepts & data model), §3.4 (key encoding), §7 (time),
and §11 (crate layout). This crate has no dependency on any other kv9 crate.
