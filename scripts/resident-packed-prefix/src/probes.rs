//! Predeclared deterministic Fisher-Yates sampling without replacement.
//! Rejection sampling avoids the modulo bias of a bounded random draw.
pub fn indices(length: usize, count: usize) -> Vec<usize> {
    assert!(count <= length);
    let mut out: Vec<_> = (0..length).collect();
    let mut state = 71u64;
    for i in (1..length).rev() {
        let bound = (i + 1) as u64;
        let threshold = bound.wrapping_neg() % bound;
        let draw = loop {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            if state >= threshold {
                break (state % bound) as usize;
            }
        };
        out.swap(i, draw);
    }
    out.truncate(count);
    out
}
