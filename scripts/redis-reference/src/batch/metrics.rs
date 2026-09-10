use super::{
    common::Histogram,
    wire::{Call, Failure},
};
use serde::Serialize;
use serde_json::{json, Value};
#[derive(Clone, Default, Serialize)]
pub struct Population {
    pub calls: u64,
    pub input_items: u64,
    pub completed_before_cutoff: u64,
    pub whole_call: Histogram,
    pub client_call: Histogram,
    pub scheduled_to_completion: Histogram,
}
#[derive(Clone, Default, Serialize)]
pub struct OperationMetrics {
    pub populations: [Population; 3],
    pub reasons: [u64; 6],
    pub dispatch_lateness: Histogram,
    pub connection_attempts: u64,
    pub connection_failures: u64,
    pub command_attempts: u64,
    pub data_failures: u64,
}
#[derive(Clone, Default)]
pub struct Metrics {
    pub operations: [OperationMetrics; 2],
}
pub struct Sample<'a> {
    pub call: &'a Call,
    pub read: bool,
    pub items: usize,
    pub whole_call_ns: u64,
    pub before_cutoff: bool,
    pub scheduled_ns: Option<u64>,
    pub lateness_ns: Option<u64>,
    pub valid: bool,
}
impl Metrics {
    pub fn record(&mut self, s: Sample<'_>) {
        let op = &mut self.operations[usize::from(!s.read)];
        let reason = match &s.call.result {
            Ok(_) if s.valid => None,
            Ok(_) => Some(Failure::DataIntegrity),
            Err(e) => Some(*e),
        };
        let outcome = if reason.is_none() {
            0
        } else if s.read {
            2
        } else {
            1
        };
        let p = &mut op.populations[outcome];
        p.calls += 1;
        p.input_items += s.items as u64;
        p.completed_before_cutoff += u64::from(s.before_cutoff);
        p.whole_call.add(s.whole_call_ns);
        p.client_call.add(s.call.elapsed_ns);
        if let Some(n) = s.scheduled_ns {
            p.scheduled_to_completion.add(n);
        }
        if let Some(n) = s.lateness_ns {
            op.dispatch_lateness.add(n);
        }
        op.reasons[reason.map_or(0, Failure::index)] += 1;
        op.connection_attempts += s.call.connection_attempts;
        op.connection_failures += s.call.connection_failures;
        op.command_attempts += s.call.command_attempts;
        op.data_failures += u64::from(!s.valid || reason.is_some_and(Failure::fatal));
    }
    pub fn merge(&mut self, b: &Self) {
        for (a, b) in self.operations.iter_mut().zip(&b.operations) {
            for (a, b) in a.populations.iter_mut().zip(&b.populations) {
                a.calls += b.calls;
                a.input_items += b.input_items;
                a.completed_before_cutoff += b.completed_before_cutoff;
                a.whole_call.merge(&b.whole_call);
                a.client_call.merge(&b.client_call);
                a.scheduled_to_completion.merge(&b.scheduled_to_completion);
            }
            for (a, b) in a.reasons.iter_mut().zip(b.reasons) {
                *a += b;
            }
            a.dispatch_lateness.merge(&b.dispatch_lateness);
            a.connection_attempts += b.connection_attempts;
            a.connection_failures += b.connection_failures;
            a.command_attempts += b.command_attempts;
            a.data_failures += b.data_failures;
        }
    }
    pub fn calls(&self) -> u64 {
        self.operations
            .iter()
            .flat_map(|o| &o.populations)
            .map(|p| p.calls)
            .sum()
    }
    pub fn valid(&self) -> bool {
        self.operations.iter().all(|o| {
            o.data_failures == 0
                && o.dispatch_lateness.valid
                && o.populations.iter().all(|p| {
                    p.whole_call.valid && p.client_call.valid && p.scheduled_to_completion.valid
                })
        })
    }
    pub fn report(&self) -> Value {
        fn h(h: &Histogram) -> Value {
            json!({"raw":h,"mean_ns":if h.valid && h.count>0{Some(h.sum_ns as f64/h.count as f64)}else{None},"p50":h.quantile(50),"p95":h.quantile(95),"p99":h.quantile(99)})
        }
        json!({"operations":["mget","mset"],"outcomes":["success","unknown_write","read_failure"],
            "reasons":["success","deadline","io","server_error","protocol","data_integrity"],"histogram_subdivisions":64,"valid":self.valid(),
            "statistics":self.operations.iter().map(|o|json!({"populations":o.populations.iter().map(|p|json!({
                "calls":p.calls,"input_items":p.input_items,"completed_before_cutoff":p.completed_before_cutoff,"whole_call":h(&p.whole_call),"client_call":h(&p.client_call),"scheduled_to_completion":h(&p.scheduled_to_completion)})).collect::<Vec<_>>(),
                "reasons":o.reasons,"dispatch_lateness":h(&o.dispatch_lateness),"connection_attempts":o.connection_attempts,"connection_failures":o.connection_failures,"command_attempts":o.command_attempts,"data_failures":o.data_failures})).collect::<Vec<_>>()})
    }
}
