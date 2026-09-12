use super::*;

fn policy() -> LeasePolicy {
    LeasePolicy {
        node: 2,
        group: 0,
        configuration: 7,
        voters: vec![1, 2, 3],
        promise_ns: 100,
        drift_ppb: 100_000,
        margin_ns: 1,
    }
}

#[test]
fn epoch_codec_rejects_truncation_extensions_version_and_invalid_identity() {
    let epoch = LeaseEpoch::successor(None, &policy()).unwrap();
    let bytes = epoch.encode();
    assert_eq!(LeaseEpoch::decode(&bytes).unwrap(), epoch);
    for end in 0..bytes.len() {
        assert!(LeaseEpoch::decode(&bytes[..end]).is_err());
    }
    let mut longer = bytes.clone();
    longer.push(0);
    assert!(LeaseEpoch::decode(&longer).is_err());
    for (at, value) in [(0, 2), (8, 0), (16, 0), (40, 0), (61, 65)] {
        let mut bad = bytes.clone();
        bad[at] = value;
        assert!(
            LeaseEpoch::decode(&bad).is_err(),
            "accepted mutation at {at}"
        );
    }
    let mut duplicate = bytes.clone();
    duplicate[70..78].copy_from_slice(&1_u64.to_be_bytes());
    assert!(LeaseEpoch::decode(&duplicate).is_err());
}

#[test]
fn every_policy_field_is_immutable_and_incarnation_cannot_wrap() {
    let p = policy();
    let epoch = LeaseEpoch::successor(None, &p).unwrap();
    let mut changed = vec![p.clone(); 8];
    changed[0].node = 1;
    changed[1].group = 1;
    changed[2].configuration += 1;
    changed[3].voters = vec![2, 3, 4];
    changed[4].promise_ns -= 1;
    changed[5].drift_ppb -= 1;
    changed[6].margin_ns = 0;
    changed[7].voters.reverse();
    for other in changed {
        assert!(LeaseEpoch::successor(Some(&epoch), &other).is_err());
    }
    let exhausted = LeaseEpoch {
        policy: p.clone(),
        incarnation: u64::MAX,
    };
    assert!(LeaseEpoch::successor(Some(&exhausted), &p).is_err());
}
