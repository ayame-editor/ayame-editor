use std::collections::BinaryHeap;

use crate::document::Document;
use crate::fields::{comparable_key_into, FieldSpec};

// ============================ TOP-N ==========================================

/// How to select the top rows.
#[derive(Clone, Debug)]
pub struct TopOptions {
    pub key_column: Option<usize>,
    pub fields: FieldSpec,
    pub numeric: bool,
    /// Keep the `n` largest keys (true) or `n` smallest (false).
    pub largest: bool,
    pub n: usize,
}

impl Default for TopOptions {
    fn default() -> Self {
        TopOptions {
            key_column: None,
            fields: FieldSpec::default(),
            numeric: false,
            largest: true,
            n: 10,
        }
    }
}

/// Return the line numbers of the top `n` rows by key, in display order
/// (largest-first for `largest`, smallest-first otherwise). Memory is `O(n)` —
/// a bounded heap, no sort of the whole input.
pub fn top_n(doc: &Document, opts: &TopOptions) -> Vec<u64> {
    use std::cmp::Reverse;
    if opts.n == 0 {
        return Vec::new();
    }
    let enc = doc.encoding();
    let total = doc.line_count();
    let n = opts.n;
    let mut field_scratch = Vec::new();
    let mut key_scratch = Vec::new();
    const BATCH: u64 = 8192;
    let mut start = 0u64;

    // `largest`: keep a min-heap of size n (evict the smallest kept key).
    // `smallest`: keep a max-heap of size n (evict the largest kept key).
    let mut min_heap: BinaryHeap<Reverse<(Vec<u8>, u64)>> = BinaryHeap::new();
    let mut max_heap: BinaryHeap<(Vec<u8>, u64)> = BinaryHeap::new();

    while start < total {
        let batch = doc.raw_line_ranges(start, BATCH);
        if batch.is_empty() {
            break;
        }
        let advanced = batch.len() as u64;
        for (ln, raw) in batch {
            comparable_key_into(
                raw,
                enc,
                opts.key_column,
                &opts.fields,
                opts.numeric,
                &mut field_scratch,
                &mut key_scratch,
            );
            if opts.largest {
                if min_heap.len() < n {
                    min_heap.push(Reverse((key_scratch.clone(), ln)));
                } else if matches!(min_heap.peek(), Some(Reverse((mk, _))) if key_scratch.as_slice() > mk.as_slice())
                {
                    min_heap.pop();
                    min_heap.push(Reverse((key_scratch.clone(), ln)));
                }
            } else if max_heap.len() < n {
                max_heap.push((key_scratch.clone(), ln));
            } else if matches!(max_heap.peek(), Some((mk, _)) if key_scratch.as_slice() < mk.as_slice())
            {
                max_heap.pop();
                max_heap.push((key_scratch.clone(), ln));
            }
        }
        start += advanced;
    }

    if opts.largest {
        let mut v: Vec<(Vec<u8>, u64)> = min_heap.into_iter().map(|Reverse(x)| x).collect();
        v.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1))); // largest key first
        v.into_iter().map(|(_, ln)| ln).collect()
    } else {
        let mut v: Vec<(Vec<u8>, u64)> = max_heap.into_iter().collect();
        v.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1))); // smallest key first
        v.into_iter().map(|(_, ln)| ln).collect()
    }
}
