use super::*;
#[test]
fn origin_census_page_cost_does_not_grow_with_retained_placement_catalog() {
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
        let revision = complete(e, &r);
        let first = q(&c, revision, 0);
        let mut initial = page(&h, None);
        initial.initial = initial::native(&c).initial;
        let view = e.snapshot().unwrap();
        let counted = Counted {
            view: view.as_ref(),
            gets: AtomicUsize::new(0),
            rows: AtomicUsize::new(0),
        };
        let plan = plan_verified(
            &counted,
            &r,
            first,
            endpoint(&r, first.source),
            None,
            &initial,
        )
        .unwrap();
        let cost = (
            counted.gets.load(Ordering::SeqCst),
            counted.rows.load(Ordering::SeqCst),
        );
        assert!(
            cost.0 < 256 && cost.1 <= 24,
            "unbounded origin census work: {cost:?}"
        );
        costs.push(cost);
        e.write(plan.into_batch()).unwrap();
        let mut last = page(&h, initial.next);
        last.initial = initial.initial.clone();
        record(e, &r, q(&c, revision, 1), initial.next, &last);
        let view = e.snapshot().unwrap();
        let counted = Counted {
            view: view.as_ref(),
            gets: AtomicUsize::new(0),
            rows: AtomicUsize::new(0),
        };
        let first_page = entries(&counted, &r, first.source, None).unwrap();
        assert_eq!(first_page.entries.len(), 4);
        // Five entry rows including lookahead; retained placement completeness
        // uses bounded point reads, not a prefix walk through the whole catalog.
        assert_eq!(counted.rows.load(Ordering::SeqCst), 5);
        assert!(counted.gets.load(Ordering::SeqCst) < 32);
    }
    assert_eq!(
        costs[0], costs[1],
        "catalog size must not change a single origin page's work"
    );
}
