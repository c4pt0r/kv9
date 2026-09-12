use super::{
    common::Histogram,
    model::{ReadApi, WriteApi},
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
    pub reasons: [u64; 7],
    pub dispatch_lateness: Histogram,
    pub connection_attempts: u64,
    pub connection_failures: u64,
    pub command_attempts: u64,
    pub confirmation_attempts: u64,
    pub replica_confirmation_replies: [u64; 3],
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
        op.confirmation_attempts += s.call.confirmation_attempts;
        if let Some(n) = s.call.confirmed_replicas {
            op.replica_confirmation_replies[usize::from(n)] += 1;
        }
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
            a.confirmation_attempts += b.confirmation_attempts;
            for (a, b) in a
                .replica_confirmation_replies
                .iter_mut()
                .zip(b.replica_confirmation_replies)
            {
                *a += b;
            }
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
        self.report_for_read_api(ReadApi::Mget)
    }
    pub fn report_for_read_api(&self, read_api: ReadApi) -> Value {
        self.report_for_apis(read_api, WriteApi::Mset)
    }
    pub fn report_for_apis(&self, read_api: ReadApi, write_api: WriteApi) -> Value {
        self.report_with_confirmation(read_api, write_api, false)
    }
    pub fn report_with_confirmation(
        &self,
        read_api: ReadApi,
        write_api: WriteApi,
        confirmation: bool,
    ) -> Value {
        fn h(h: &Histogram) -> Value {
            json!({"raw":h,"mean_ns":if h.valid && h.count>0{Some(h.sum_ns as f64/h.count as f64)}else{None},"p50":h.quantile(50),"p95":h.quantile(95),"p99":h.quantile(99)})
        }
        let read_operation = match read_api {
            ReadApi::Mget => "mget",
            ReadApi::Get => "get",
        };
        let write_operation = match write_api {
            WriteApi::Mset => "mset",
            WriteApi::Set => "set",
        };
        let mut report = json!({"operations":[read_operation,write_operation],"outcomes":["success","unknown_write","read_failure"],
            "reasons":["success","deadline","io","server_error","protocol","data_integrity","replication_shortfall"],"histogram_subdivisions":64,"valid":self.valid(),
            "statistics":self.operations.iter().map(|o|json!({"populations":o.populations.iter().map(|p|json!({
                "calls":p.calls,"input_items":p.input_items,"completed_before_cutoff":p.completed_before_cutoff,"whole_call":h(&p.whole_call),"client_call":h(&p.client_call),"scheduled_to_completion":h(&p.scheduled_to_completion)})).collect::<Vec<_>>(),
                "reasons":o.reasons,"dispatch_lateness":h(&o.dispatch_lateness),"connection_attempts":o.connection_attempts,"connection_failures":o.connection_failures,"command_attempts":o.command_attempts,
                "confirmation_attempts":o.confirmation_attempts,"replica_confirmation_replies":o.replica_confirmation_replies,
                "total_resp_command_attempts":o.command_attempts+o.confirmation_attempts,"data_failures":o.data_failures})).collect::<Vec<_>>()});
        // Versions 1..=3 retain their exact metric vocabulary and field shape.
        if !confirmation {
            report["reasons"].as_array_mut().unwrap().truncate(6);
            for op in report["statistics"].as_array_mut().unwrap() {
                op["reasons"].as_array_mut().unwrap().truncate(6);
                let fields = op.as_object_mut().unwrap();
                fields.remove("confirmation_attempts");
                fields.remove("replica_confirmation_replies");
                fields.remove("total_resp_command_attempts");
            }
        }
        report
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failed_get_keeps_one_whole_read_failure_without_write_or_replay() {
        let call = Call {
            result: Err(Failure::Deadline),
            elapsed_ns: 100,
            connection_attempts: 1,
            connection_failures: 0,
            command_attempts: 1,
            confirmation_attempts: 0,
            confirmed_replicas: None,
        };
        let mut metrics = Metrics::default();
        metrics.record(Sample {
            call: &call,
            read: true,
            items: 1,
            whole_call_ns: 120,
            before_cutoff: false,
            scheduled_ns: Some(150),
            lateness_ns: Some(30),
            valid: true,
        });
        let read = &metrics.operations[0];
        assert_eq!(metrics.calls(), 1);
        assert_eq!(read.populations[0].calls, 0);
        assert_eq!(read.populations[1].calls, 0);
        assert_eq!(
            (read.populations[2].calls, read.populations[2].input_items),
            (1, 1)
        );
        assert_eq!(read.populations[2].whole_call.count, 1);
        assert_eq!(read.populations[2].whole_call.sum_ns, 120);
        assert_eq!(read.populations[2].scheduled_to_completion.sum_ns, 150);
        assert_eq!(read.command_attempts, 1);
        assert!(metrics.operations[1]
            .populations
            .iter()
            .all(|p| p.calls == 0));
        assert_eq!(
            metrics.report_for_read_api(ReadApi::Get)["operations"],
            json!(["get", "mset"])
        );
        assert_eq!(metrics.report()["operations"], json!(["mget", "mset"]));
    }

    #[test]
    fn failed_set_is_one_unknown_write_and_legacy_labels_stay_unchanged() {
        let call = Call {
            result: Err(Failure::Deadline),
            elapsed_ns: 100,
            connection_attempts: 1,
            connection_failures: 0,
            command_attempts: 1,
            confirmation_attempts: 0,
            confirmed_replicas: None,
        };
        let mut metrics = Metrics::default();
        metrics.record(Sample {
            call: &call,
            read: false,
            items: 1,
            whole_call_ns: 120,
            before_cutoff: false,
            scheduled_ns: Some(150),
            lateness_ns: Some(30),
            valid: true,
        });
        let write = &metrics.operations[1];
        assert_eq!(metrics.calls(), 1);
        assert_eq!(write.populations[0].calls, 0);
        assert_eq!(write.populations[1].calls, 1);
        assert_eq!(write.populations[1].input_items, 1);
        assert_eq!(write.populations[1].whole_call.sum_ns, 120);
        assert_eq!(write.populations[1].scheduled_to_completion.sum_ns, 150);
        assert_eq!(write.populations[2].calls, 0);
        assert_eq!(write.command_attempts, 1);
        assert_eq!(write.reasons, [0, 1, 0, 0, 0, 0, 0]);
        assert_eq!(
            metrics.report_for_apis(ReadApi::Get, WriteApi::Set)["operations"],
            json!(["get", "set"])
        );
        assert_eq!(metrics.report()["operations"], json!(["mget", "mset"]));
        assert_eq!(
            metrics.report_for_read_api(ReadApi::Get)["operations"],
            json!(["get", "mset"])
        );
    }
}
