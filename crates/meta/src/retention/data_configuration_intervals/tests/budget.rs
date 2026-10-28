use super::*;
#[test]
fn interval_projection_page_cost_is_independent_of_retained_catalog_size() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    struct Counted<'a> {
        view: &'a dyn ReadView,
        gets: AtomicUsize,
        rows: AtomicUsize,
    }
    impl ReadView for Counted<'_> {
        fn scan(
            &self,
            cf: ColumnFamily,
            start: &[u8],
            end: &[u8],
            limit: usize,
        ) -> Result<Vec<kv9_engine::ScanEntry>> {
            let rows = self.view.scan(cf, start, end, limit)?;
            self.rows.fetch_add(rows.len(), Ordering::SeqCst);
            Ok(rows)
        }
        fn seek_le(&self, cf: ColumnFamily, key: &[u8]) -> Result<Option<kv9_engine::ScanEntry>> {
            self.gets.fetch_add(1, Ordering::SeqCst);
            self.view.seek_le(cf, key)
        }
        fn get(&self, cf: ColumnFamily, key: &[u8]) -> Result<Option<Vec<u8>>> {
            self.gets.fetch_add(1, Ordering::SeqCst);
            self.view.get(cf, key)
        }
        fn iter<'a>(
            &'a self,
            cf: ColumnFamily,
            start: &[u8],
            end: &[u8],
        ) -> Result<Box<dyn Iterator<Item = Result<kv9_engine::ScanEntry>> + 'a>> {
            Ok(Box::new(self.view.iter(cf, start, end)?.inspect(|_| {
                self.rows.fetch_add(1, Ordering::SeqCst);
            })))
        }
        fn iter_rev<'a>(
            &'a self,
            cf: ColumnFamily,
            start: &[u8],
            end: &[u8],
        ) -> Result<Box<dyn Iterator<Item = Result<kv9_engine::ScanEntry>> + 'a>> {
            Ok(Box::new(self.view.iter_rev(cf, start, end)?.inspect(
                |_| {
                    self.rows.fetch_add(1, Ordering::SeqCst);
                },
            )))
        }
    }

    let mut costs = Vec::new();
    for size in [13, 113] {
        let (s, r) = fixture();
        let c = group(&s, 1);
        let h = history(&s, &c, &r);
        for n in 10..10 + size {
            group(&s, n);
        }
        let e = s.engine().as_ref();
        let source = seed(e, &r, &c, &h, true);
        let request = next(e.snapshot().unwrap().as_ref(), &r, source)
            .unwrap()
            .unwrap();
        let view = e.snapshot().unwrap();
        let counted = Counted {
            view: view.as_ref(),
            gets: AtomicUsize::new(0),
            rows: AtomicUsize::new(0),
        };
        let plan = plan_retention(
            &counted,
            &r,
            &LedgerRequest::AdvanceConfigurationIntervals(request),
        )
        .unwrap();
        let cost = (
            counted.gets.load(Ordering::SeqCst),
            counted.rows.load(Ordering::SeqCst),
        );
        assert!(
            cost.0 < 256 && cost.1 <= 8,
            "unbounded projection work: {cost:?}"
        );
        costs.push(cost);
        e.write(plan.into_batch()).unwrap();
        finish(e, &r, source);
        let view = e.snapshot().unwrap();
        let counted = Counted {
            view: view.as_ref(),
            gets: AtomicUsize::new(0),
            rows: AtomicUsize::new(0),
        };
        let page = intervals(&counted, &r, source, None).unwrap();
        assert_eq!(page.intervals.len(), 4);
        assert!(counted.gets.load(Ordering::SeqCst) < 96);
        assert!(counted.rows.load(Ordering::SeqCst) <= 8);
    }
    assert_eq!(costs[0], costs[1]);
}
#[test]
fn intervals_preserve_large_joint_roles_and_reject_unbounded_member_decode() {
    let (s, r) = fixture();
    let c = group(&s, 1);
    let e = s.engine().as_ref();
    let state = ConfigurationState {
        voters: (1..=1024).collect(),
        outgoing: (1025..=2048).collect(),
        learners: (2049..=3072).collect(),
        learners_next: (1025..=2048).collect(),
        auto_leave: true,
    };
    let h: Vec<_> = (1..=4)
        .map(|ordinal| ConfigurationRecord {
            ordinal,
            kind: RecordKind::Applied { index: ordinal },
            state: Some(state.clone()),
            origin: None,
        })
        .collect();
    let source = seed(e, &r, &c, &h, false);
    finish(e, &r, source);
    let view = e.snapshot().unwrap();
    let p = planner(view.as_ref(), &r).unwrap();
    let first = read_entry(&p, source, 1).unwrap().unwrap();
    assert_eq!(first.members.as_ref().unwrap().len(), 3072);
    assert!(first
        .members
        .as_ref()
        .unwrap()
        .iter()
        .all(|m| m.store.is_none()));
    assert!(first
        .members
        .as_ref()
        .unwrap()
        .iter()
        .filter(|m| m.node.0 >= 1025 && m.node.0 <= 2048)
        .all(|m| m.roles == 2 | 8));
    let bytes = first.encode(r.digest(), source).unwrap();
    assert_eq!(Entry::decode(&p, source, 1, &bytes).unwrap(), first);
    let mut bad = bytes[..bytes.len() - 32].to_vec();
    bad[163..165].copy_from_slice(&u16::MAX.to_be_bytes());
    assert!(Entry::decode(&p, source, 1, &seal(bad)).is_err());
}
