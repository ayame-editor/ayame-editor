use std::collections::BinaryHeap;

use crate::document::Document;
use crate::fields::{comparable_key_into, FieldSpec};
use crate::Result;

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

/// One selected row: its logical record number and the absolute byte range of
/// its raw bytes (terminator included in `raw_end`). In CSV mode a record can
/// span physical lines (#199), so `record` is NOT a viewport line number —
/// callers display the row through the byte range, never through
/// `Document::line(record)`.
#[derive(Clone, Copy, Debug)]
pub struct TopRow {
    pub record: u64,
    pub start: u64,
    pub raw_end: u64,
}

/// Return the top `n` rows by key, in display order (largest-first for
/// `largest`, smallest-first otherwise). Memory is `O(n)` — a bounded heap,
/// no sort of the whole input.
pub fn top_n(doc: &Document, opts: &TopOptions) -> Result<Vec<TopRow>> {
    use std::cmp::Reverse;
    if opts.n == 0 {
        return Ok(Vec::new());
    }
    let enc = doc.encoding();
    let n = opts.n;
    let mut field_scratch = Vec::new();
    let mut key_scratch = Vec::new();

    // `largest`: keep a min-heap of size n (evict the smallest kept key).
    // `smallest`: keep a max-heap of size n (evict the largest kept key).
    type Entry = (Vec<u8>, u64, u64, u64); // (key, record, start, raw_end)
    let mut min_heap: BinaryHeap<Reverse<Entry>> = BinaryHeap::new();
    let mut max_heap: BinaryHeap<Entry> = BinaryHeap::new();

    // Logical records (RFC-4180 in CSV mode; #199).
    super::common::try_for_each_record(
        doc,
        &opts.fields,
        |record, raw, start, raw_end| {
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
                    min_heap.push(Reverse((key_scratch.clone(), record, start, raw_end)));
                } else if matches!(min_heap.peek(), Some(Reverse((mk, ..))) if key_scratch.as_slice() > mk.as_slice())
                {
                    min_heap.pop();
                    min_heap.push(Reverse((key_scratch.clone(), record, start, raw_end)));
                }
            } else if max_heap.len() < n {
                max_heap.push((key_scratch.clone(), record, start, raw_end));
            } else if matches!(max_heap.peek(), Some((mk, ..)) if key_scratch.as_slice() < mk.as_slice())
            {
                max_heap.pop();
                max_heap.push((key_scratch.clone(), record, start, raw_end));
            }
            Ok(())
        },
        |_| {},
    )?;

    let row = |(_, record, start, raw_end): Entry| TopRow {
        record,
        start,
        raw_end,
    };
    Ok(if opts.largest {
        let mut v: Vec<Entry> = min_heap.into_iter().map(|Reverse(x)| x).collect();
        v.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1))); // largest key first
        v.into_iter().map(row).collect()
    } else {
        let mut v: Vec<Entry> = max_heap.into_iter().collect();
        v.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1))); // smallest key first
        v.into_iter().map(row).collect()
    })
}
