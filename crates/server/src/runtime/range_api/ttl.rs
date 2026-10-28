//! Public TTL requests own a single deadline and a fixed proposal. Reads commit
//! a qualified observation, await its exact receipt, then establish ReadIndex.
use super::*;
use crate::raw_ttl::{Operation, Preparation, Reply, MAX_SCAN_EXAMINED};
use kv9_common::raw_ttl::{
    clock::ClockRefusal, ClockWatermark, ExpiryPlan, TtlRejection, CLOCK_KEY,
};
use kv9_raft::command::{RawTtlAction, RawTtlCommand, RawTtlMutation};
use kv9_raw::raw::ttl::TtlRead;

fn remaining(deadline: Instant) -> Result<Duration> {
    deadline
        .checked_duration_since(Instant::now())
        .filter(|b| !b.is_zero())
        .ok_or_else(|| Error::Raft("Raw TTL request deadline expired".into()))
}

impl RawGroup {
    fn ttl_authorize(
        &self,
        view: &dyn ReadView,
        ctx: &RequestContext,
        op: &Operation,
    ) -> Result<ClockWatermark> {
        let span = match op {
            Operation::Capabilities => KeySpan::Point(&self.binding.start),
            Operation::Get { keys } => KeySpan::BatchKeys(keys),
            Operation::Put { pairs, .. } => KeySpan::BatchPairs(pairs),
            Operation::Delete { key } => KeySpan::Point(key),
            Operation::Scan { start, end, .. } | Operation::DeleteRange { start, end, .. } => {
                KeySpan::Range { start, end }
            }
        };
        self.authorize_range(view, ctx, span)?;
        let bytes = view
            .get(ColumnFamily::Default, CLOCK_KEY)?
            .ok_or(Error::RawTtlRejected {
                region: self.binding.region,
                reason: TtlRejection::NotEnabled,
            })?;
        ClockWatermark::decode(&bytes)
    }

    pub(super) fn prepare_ttl(
        self: Arc<Self>,
        ctx: RequestContext,
        op: Operation,
        deadline: Instant,
    ) -> Preparation {
        Box::new(move || {
            op.validate()?;
            // Verify applied state after leadership confirmation, before sampling.
            self.driver.read_barrier(remaining(deadline)?)?;
            let view = self.engine.snapshot()?;
            let committed = self.ttl_authorize(view.as_ref(), &ctx, &op)?;
            if let Operation::DeleteRange { start, end, limit } = &op {
                self.write_pressure()?;
                let read = LeaderRead::new(view.as_ref(), true, None)?;
                let page = TtlRead::from_leader(&read)?.delete_page(
                    ctx.keyspace,
                    start,
                    end,
                    *limit,
                    crate::client::MAX_MESSAGE_BYTES - 256,
                )?;
                drop(view);
                remaining(deadline)?;
                let processed_keys = page.keys.len();
                if processed_keys == 0 {
                    return Ok(Box::pin(async {
                        Ok(Reply::Deleted {
                            processed_keys: 0,
                            applied: None,
                            next: None,
                        })
                    }));
                }
                let command = RawTtlCommand::new(
                    &self.binding,
                    RawTtlAction::Write {
                        plan: ExpiryPlan::permanent(committed),
                        mutations: page
                            .keys
                            .into_iter()
                            .map(|key| RawTtlMutation::Delete { key })
                            .collect(),
                    },
                )?;
                remaining(deadline)?;
                let pending = self.driver.propose_raw_ttl_command(command, deadline)?;
                return Ok(Box::pin(async move {
                    let at = settle_proposal(pending.proposed_at(), pending.wait().await, true)?
                        .expect("TTL delete does not retry");
                    Ok(Reply::Deleted {
                        processed_keys,
                        applied: Some(at.into()),
                        next: page.next,
                    })
                }));
            }
            drop(view);
            if matches!(op, Operation::Capabilities) {
                return Ok(Box::pin(async { Ok(Reply::Capabilities) }));
            }
            // Reads also append Observe. They must not bypass retained-log bounds.
            self.write_pressure()?;
            let ttl = match &op {
                Operation::Put { ttl_secs, .. } => *ttl_secs,
                _ => None,
            };
            let plan = if op.is_read() || ttl.is_some_and(|s| s > 0) {
                let sample = self
                    .clock
                    .as_ref()
                    .ok_or(Error::RawTtlClock {
                        reason: ClockRefusal::Unavailable,
                    })?
                    .sample()
                    .map_err(|reason| Error::RawTtlClock { reason })?;
                sample.advance(committed).map_err(|_| Error::RawTtlClock {
                    reason: ClockRefusal::BehindWatermark,
                })?;
                ExpiryPlan::resolve(ttl, sample, committed)?
            } else if ttl == Some(0) {
                ExpiryPlan::immediate(committed)
            } else {
                ExpiryPlan::permanent(committed)
            };
            let action = match &op {
                Operation::Put { pairs, .. } => RawTtlAction::Write {
                    plan,
                    mutations: pairs
                        .iter()
                        .map(|(key, value)| RawTtlMutation::Put {
                            key: key.clone(),
                            value: value.clone(),
                        })
                        .collect(),
                },
                Operation::Delete { key } => RawTtlAction::Write {
                    plan,
                    mutations: vec![RawTtlMutation::Delete { key: key.clone() }],
                },
                _ => RawTtlAction::Observe {
                    observed: plan.observed(),
                },
            };
            let command = RawTtlCommand::new(&self.binding, action)?;
            remaining(deadline)?;
            let pending = self.driver.propose_raw_ttl_command(command, deadline)?;
            Ok(Box::pin(async move {
                let proposed = pending.proposed_at();
                // Never recalculate a duration or automatically re-propose after
                // replacement. Unknown and replaced writes are terminal here.
                let at = settle_proposal(proposed, pending.wait().await, true)?
                    .expect("exhausted retries cannot return a retry decision");
                if !op.is_read() {
                    return Ok(Reply::from(at));
                }
                self.driver.read_barrier_async(remaining(deadline)?).await?;
                tokio::task::spawn_blocking(move || {
                    remaining(deadline)?;
                    let view = self.engine.snapshot()?;
                    let watermark = self.ttl_authorize(view.as_ref(), &ctx, &op)?;
                    if watermark < plan.observed() {
                        return Err(Error::Engine(
                            "TTL read snapshot precedes its committed observation".into(),
                        ));
                    }
                    let read = TtlRead::from_leader(&LeaderRead::new(view.as_ref(), true, None)?)?;
                    let result = match op {
                        Operation::Get { keys } => Reply::Values {
                            values: read.batch_get(ctx.keyspace, &keys)?,
                        },
                        Operation::Scan { start, end, limit } => {
                            let page = read.scan_page_with_budget(
                                ctx.keyspace,
                                &start,
                                &end,
                                kv9_raw::raw::ttl::ScanBudget {
                                    rows: limit,
                                    examined: MAX_SCAN_EXAMINED,
                                    result_bytes: crate::client::MAX_MESSAGE_BYTES
                                        - (crate::raw_ttl::MAX_RANGE_BOUND_BYTES + 1)
                                        - 256,
                                },
                            )?;
                            Reply::Page {
                                rows: page.rows,
                                next: page.next,
                                examined: page.examined,
                            }
                        }
                        _ => unreachable!("only reads reach the snapshot path"),
                    };
                    remaining(deadline)?;
                    Ok(result)
                })
                .await
                .map_err(|e| Error::Raft(format!("TTL read worker failed: {e}")))?
            }))
        })
    }
}

#[cfg(test)]
mod tests;
