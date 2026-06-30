//! Out-of-core data operations over a [`Document`].
//!
//! These ops are bounded by an explicit **memory budget** and spill the
//! overflow to disk — the project's core trade-off (spend disk to bound memory
//! and stay stable). The first op is an external merge sort; group-by/top-n
//! follow the same shape.
//!
//! Sorting produces an **ordering**: a file of `u64` line numbers in sorted
//! order. The editor can page through that ordering via the existing sparse
//! fetch path, so a sorted view at Ayame's minimum ten-billion-line scale never
//! materializes the lines themselves — only their order.

use std::cmp::Ordering;
use std::collections::BinaryHeap;
use std::fs::{self, File};
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};

use rayon::prelude::*;

use crate::document::Document;
use crate::encoding::Encoding;
use crate::{Error, Result};

/// How a key/value field is located within a line.
#[derive(Clone, Copy, Debug)]
pub struct FieldSpec {
    /// Field delimiter (e.g. `,` or `\t`).
    pub delimiter: u8,
    /// Quote character for RFC-4180 parsing (only used when `csv` is true).
    pub quote: u8,
    /// When true, split with a real CSV parser (quoted fields may contain the
    /// delimiter; `""` is an escaped quote). When false, split on raw delimiter
    /// bytes (faster; correct for clean TSV/CSV without quoting).
    pub csv: bool,
}

impl Default for FieldSpec {
    fn default() -> Self {
        FieldSpec {
            delimiter: b',',
            quote: b'"',
            csv: false,
        }
    }
}

/// How to sort.
#[derive(Clone, Debug)]
pub struct SortOptions {
    /// 1-based field index to sort on; `None` sorts on the whole line.
    pub key_column: Option<usize>,
    /// How to locate the key field.
    pub fields: FieldSpec,
    /// Parse the key as a number (order-preserving); else codepoint order.
    pub numeric: bool,
    /// Descending instead of ascending.
    pub reverse: bool,
    /// Soft memory budget (bytes) for run generation before spilling.
    pub budget_bytes: usize,
    /// Directory for spill runs and the ordering file (created if missing).
    pub spill_dir: PathBuf,
}

impl Default for SortOptions {
    fn default() -> Self {
        SortOptions {
            key_column: None,
            fields: FieldSpec::default(),
            numeric: false,
            reverse: false,
            budget_bytes: 256 * 1024 * 1024,
            spill_dir: std::env::temp_dir().join("ayame-sort"),
        }
    }
}

/// Outcome of [`sort`].
#[derive(Clone, Debug)]
pub struct SortResult {
    /// File of `u64` LE line numbers in sorted order.
    pub ordering_path: PathBuf,
    pub line_count: u64,
    /// Number of spilled sorted runs the merge consumed.
    pub runs: usize,
    /// Total bytes written to spill runs (a proxy for disk used).
    pub spill_bytes: u64,
}

/// Sort `doc` by the configured key, returning an ordering file.
///
/// Phase 1 streams the document in line-aligned batches, building comparable
/// byte keys, and spills sorted `(key, line_no)` runs whenever the in-memory
/// buffer reaches `budget_bytes`. Phase 2 k-way-merges the runs with a heap,
/// writing the sorted line numbers. Peak memory is `O(budget + runs)`.
pub fn sort(doc: &Document, opts: &SortOptions) -> Result<SortResult> {
    fs::create_dir_all(&opts.spill_dir)?;

    // ---- phase 1: run generation ----------------------------------------
    let total = doc.line_count();
    let enc = doc.encoding();
    let mut buffer: Vec<(Vec<u8>, u64)> = Vec::new();
    let mut buffered_bytes: usize = 0;
    let mut runs: Vec<PathBuf> = Vec::new();
    let mut spill_bytes: u64 = 0;

    const BATCH: u64 = 8192;
    let mut start = 0u64;
    let mut scratch = Vec::new();
    while start < total {
        let batch = doc.raw_line_ranges(start, BATCH);
        if batch.is_empty() {
            break;
        }
        let advanced = batch.len() as u64;
        for (line_no, raw) in batch {
            let key = make_key(raw, enc, opts, &mut scratch);
            buffered_bytes += key.len() + 40; // key + Vec/tuple overhead estimate
            buffer.push((key, line_no));
            if buffered_bytes >= opts.budget_bytes {
                spill_bytes += spill_run(&mut buffer, opts.reverse, &opts.spill_dir, &mut runs)?;
                buffered_bytes = 0;
            }
        }
        start += advanced;
    }
    if !buffer.is_empty() {
        spill_bytes += spill_run(&mut buffer, opts.reverse, &opts.spill_dir, &mut runs)?;
    }

    // ---- phase 2: k-way merge -------------------------------------------
    let ordering_path = opts.spill_dir.join("ordering.bin");
    let merged = merge_runs(&runs, opts.reverse, &ordering_path)?;
    // Runs are consumed; delete them so only the ordering remains.
    for r in &runs {
        let _ = fs::remove_file(r);
    }

    Ok(SortResult {
        ordering_path,
        line_count: merged,
        runs: runs.len(),
        spill_bytes,
    })
}

/// Read a sorted ordering file as a stream of `u64` line numbers.
pub struct OrderingReader {
    r: BufReader<File>,
}

impl OrderingReader {
    pub fn open(path: &Path) -> Result<OrderingReader> {
        Ok(OrderingReader {
            r: BufReader::new(File::open(path)?),
        })
    }

    /// Next line number, or `None` at end of file.
    pub fn next_line(&mut self) -> Result<Option<u64>> {
        let mut b = [0u8; 8];
        match read_full(&mut self.r, &mut b)? {
            true => Ok(Some(u64::from_le_bytes(b))),
            false => Ok(None),
        }
    }
}

// ---- key construction --------------------------------------------------------

fn make_key(raw: &[u8], enc: Encoding, opts: &SortOptions, scratch: &mut Vec<u8>) -> Vec<u8> {
    comparable_key(
        raw,
        enc,
        opts.key_column,
        &opts.fields,
        opts.numeric,
        scratch,
    )
}

/// Build a byte key whose `Ord` matches the desired sort order: an
/// order-preserving 8-byte encoding for numeric keys, else the field decoded to
/// UTF-8 (byte order == code-point order). Shared by sort and top-n.
fn comparable_key(
    raw: &[u8],
    enc: Encoding,
    col: Option<usize>,
    spec: &FieldSpec,
    numeric: bool,
    scratch: &mut Vec<u8>,
) -> Vec<u8> {
    let field = extract_field(raw, col, spec, scratch);
    if numeric {
        let v = enc
            .decode_line(field)
            .trim()
            .parse::<f64>()
            .unwrap_or(f64::INFINITY);
        f64_order_key(v).to_vec()
    } else {
        enc.decode_line(field).into_bytes()
    }
}

/// Extract the key/value field from a line. Returns a borrow of `raw` for the
/// fast (whole-line or raw-split) paths, or of `scratch` after CSV parsing.
fn extract_field<'a>(
    raw: &'a [u8],
    col: Option<usize>,
    spec: &FieldSpec,
    scratch: &'a mut Vec<u8>,
) -> &'a [u8] {
    match col {
        None => raw,
        Some(c) if spec.csv => {
            csv_nth_field(raw, spec.delimiter, spec.quote, c, scratch);
            &scratch[..]
        }
        Some(c) => nth_field(raw, spec.delimiter, c),
    }
}

/// 1-based field by raw delimiter byte (no quote handling); empty if out of range.
fn nth_field(raw: &[u8], delim: u8, col: usize) -> &[u8] {
    if col == 0 {
        return raw;
    }
    let mut idx = 1;
    let mut field_start = 0usize;
    for (i, &b) in raw.iter().enumerate() {
        if b == delim {
            if idx == col {
                return &raw[field_start..i];
            }
            idx += 1;
            field_start = i + 1;
        }
    }
    if idx == col {
        &raw[field_start..]
    } else {
        &[]
    }
}

/// RFC-4180-aware extraction of the 1-based `col` field of a single record into
/// `out` (cleared first), unescaping quotes. Uses `csv-core` (allocation-free).
///
/// NOTE: one physical line == one record. A quoted field with an *embedded
/// newline* would have already been split by the line index, so embedded
/// newlines in quoted fields are not supported (see DESIGN / ROADMAP). Quoted
/// delimiters and `""` escapes within a line are handled correctly.
fn csv_nth_field(raw: &[u8], delim: u8, quote: u8, col: usize, out: &mut Vec<u8>) {
    out.clear();
    if col == 0 {
        out.extend_from_slice(raw);
        return;
    }
    let mut rdr = csv_core::ReaderBuilder::new()
        .delimiter(delim)
        .quote(quote)
        .build();
    let mut input = raw;
    let mut buf = [0u8; 512];
    let mut idx = 1usize;
    let mut flushed = false;
    loop {
        let (res, nin, nout) = rdr.read_field(input, &mut buf);
        input = &input[nin..];
        if idx == col {
            out.extend_from_slice(&buf[..nout]);
        }
        match res {
            csv_core::ReadFieldResult::InputEmpty => {
                if input.is_empty() {
                    if flushed {
                        break;
                    }
                    flushed = true; // one more call with empty input flushes the final field
                }
            }
            csv_core::ReadFieldResult::OutputFull => {} // same field continues into buf again
            csv_core::ReadFieldResult::Field { record_end } => {
                if idx == col {
                    break;
                }
                idx += 1;
                if record_end {
                    break;
                }
            }
            csv_core::ReadFieldResult::End => break,
        }
    }
}

/// Map an f64 to an 8-byte big-endian key whose unsigned byte order equals the
/// numeric order of the original value (handles negatives and -0.0).
fn f64_order_key(x: f64) -> [u8; 8] {
    let bits = x.to_bits();
    let ord = if bits & 0x8000_0000_0000_0000 != 0 {
        !bits // negative: flip all bits
    } else {
        bits ^ 0x8000_0000_0000_0000 // non-negative: flip sign bit
    };
    ord.to_be_bytes()
}

// ---- run generation / merge --------------------------------------------------

fn spill_run(
    records: &mut Vec<(Vec<u8>, u64)>,
    reverse: bool,
    dir: &Path,
    runs: &mut Vec<PathBuf>,
) -> Result<u64> {
    // Sort the run in the same direction the merge will consume it; ties break
    // on line number so the sort is stable.
    records.par_sort_unstable_by(|a, b| {
        let k = a.0.cmp(&b.0);
        let k = if reverse { k.reverse() } else { k };
        k.then_with(|| a.1.cmp(&b.1))
    });

    let path = dir.join(format!("run{:05}.bin", runs.len()));
    let mut w = BufWriter::new(File::create(&path)?);
    let mut bytes = 0u64;
    for (key, line) in records.iter() {
        let len = key.len() as u32;
        w.write_all(&len.to_le_bytes())?;
        w.write_all(key)?;
        w.write_all(&line.to_le_bytes())?;
        bytes += 4 + key.len() as u64 + 8;
    }
    w.flush()?;
    runs.push(path);
    records.clear();
    Ok(bytes)
}

struct RunReader {
    r: BufReader<File>,
}

impl RunReader {
    fn open(path: &Path) -> Result<RunReader> {
        Ok(RunReader {
            r: BufReader::new(File::open(path)?),
        })
    }

    fn next_record(&mut self) -> Result<Option<(Vec<u8>, u64)>> {
        let mut len_b = [0u8; 4];
        if !read_full(&mut self.r, &mut len_b)? {
            return Ok(None);
        }
        let len = u32::from_le_bytes(len_b) as usize;
        let mut key = vec![0u8; len];
        self.r.read_exact(&mut key)?;
        let mut line_b = [0u8; 8];
        self.r.read_exact(&mut line_b)?;
        Ok(Some((key, u64::from_le_bytes(line_b))))
    }
}

/// Heap element. `Ord` is arranged so `BinaryHeap` (a max-heap) pops whichever
/// record should be emitted next; ties prefer the smaller line number (stable).
struct HeapItem {
    key: Vec<u8>,
    line_no: u64,
    run: usize,
    reverse: bool,
}

impl PartialEq for HeapItem {
    fn eq(&self, o: &Self) -> bool {
        self.cmp(o) == Ordering::Equal
    }
}
impl Eq for HeapItem {}
impl PartialOrd for HeapItem {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}
impl Ord for HeapItem {
    fn cmp(&self, o: &Self) -> Ordering {
        let key_ord = self.key.cmp(&o.key);
        // Ascending: the smaller key must be "greater" so the max-heap pops it.
        let primary = if self.reverse {
            key_ord
        } else {
            key_ord.reverse()
        };
        // Tie-break: smaller line number emitted first => it must be "greater".
        primary.then_with(|| o.line_no.cmp(&self.line_no))
    }
}

fn merge_runs(runs: &[PathBuf], reverse: bool, ordering_path: &Path) -> Result<u64> {
    let mut readers: Vec<RunReader> = runs
        .iter()
        .map(|p| RunReader::open(p))
        .collect::<Result<_>>()?;
    let mut heap: BinaryHeap<HeapItem> = BinaryHeap::with_capacity(readers.len());
    for (i, rr) in readers.iter_mut().enumerate() {
        if let Some((key, line_no)) = rr.next_record()? {
            heap.push(HeapItem {
                key,
                line_no,
                run: i,
                reverse,
            });
        }
    }

    let mut out = BufWriter::new(File::create(ordering_path)?);
    let mut count = 0u64;
    while let Some(item) = heap.pop() {
        out.write_all(&item.line_no.to_le_bytes())?;
        count += 1;
        if let Some((key, line_no)) = readers[item.run].next_record()? {
            heap.push(HeapItem {
                key,
                line_no,
                run: item.run,
                reverse,
            });
        }
    }
    out.flush()?;
    Ok(count)
}

// ======================= group-by (hash aggregation) =========================

/// How to group and aggregate.
#[derive(Clone, Debug)]
pub struct GroupOptions {
    /// 1-based key field; `None` groups on the whole line.
    pub key_column: Option<usize>,
    /// 1-based numeric value field for sum/min/max/avg; `None` = count only.
    pub value_column: Option<usize>,
    /// How to locate key/value fields.
    pub fields: FieldSpec,
    /// Memory budget for the in-memory aggregation map before spilling.
    pub budget_bytes: usize,
    pub spill_dir: PathBuf,
}

impl Default for GroupOptions {
    fn default() -> Self {
        GroupOptions {
            key_column: None,
            value_column: None,
            fields: FieldSpec::default(),
            budget_bytes: 256 * 1024 * 1024,
            spill_dir: std::env::temp_dir().join("ayame-group"),
        }
    }
}

/// One aggregated group emitted by [`group`].
#[derive(Clone, Debug)]
pub struct GroupRow {
    pub key: Vec<u8>,
    pub count: u64,
    /// How many rows had a parseable numeric value (for avg).
    pub numeric_count: u64,
    pub sum: f64,
    pub min: f64,
    pub max: f64,
}

impl GroupRow {
    pub fn avg(&self) -> Option<f64> {
        if self.numeric_count > 0 {
            Some(self.sum / self.numeric_count as f64)
        } else {
            None
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct GroupStats {
    pub groups: u64,
    pub runs: usize,
    pub spill_bytes: u64,
}

#[derive(Clone, Copy)]
struct Acc {
    count: u64,
    ncount: u64,
    sum: f64,
    min: f64,
    max: f64,
}

impl Acc {
    fn new() -> Acc {
        Acc {
            count: 0,
            ncount: 0,
            sum: 0.0,
            min: f64::INFINITY,
            max: f64::NEG_INFINITY,
        }
    }
    fn add(&mut self, v: Option<f64>) {
        self.count += 1;
        if let Some(x) = v {
            self.ncount += 1;
            self.sum += x;
            self.min = self.min.min(x);
            self.max = self.max.max(x);
        }
    }
    fn combine(&mut self, o: &Acc) {
        self.count += o.count;
        self.ncount += o.ncount;
        self.sum += o.sum;
        self.min = self.min.min(o.min);
        self.max = self.max.max(o.max);
    }
}

/// Group `doc` by the key field and aggregate, calling `emit` once per group in
/// ascending key order. Keeps an in-memory map up to `budget_bytes`; when it
/// overflows it spills a sorted partial-aggregate run and continues, then
/// k-way-merges the runs combining equal keys. The common case (few groups)
/// never touches disk; unbounded cardinality stays within budget.
pub fn group(
    doc: &Document,
    opts: &GroupOptions,
    mut emit: impl FnMut(&GroupRow),
) -> Result<GroupStats> {
    use std::collections::HashMap;
    fs::create_dir_all(&opts.spill_dir)?;
    let enc = doc.encoding();
    let total = doc.line_count();

    let mut map: HashMap<Vec<u8>, Acc> = HashMap::new();
    let mut map_bytes = 0usize;
    let mut runs: Vec<PathBuf> = Vec::new();
    let mut spill_bytes = 0u64;

    const BATCH: u64 = 8192;
    let mut start = 0u64;
    let mut scratch = Vec::new();
    while start < total {
        let batch = doc.raw_line_ranges(start, BATCH);
        if batch.is_empty() {
            break;
        }
        let advanced = batch.len() as u64;
        for (_ln, raw) in batch {
            let key = key_bytes(raw, enc, opts.key_column, &opts.fields, &mut scratch);
            let value = opts.value_column.and_then(|c| {
                let f = extract_field(raw, Some(c), &opts.fields, &mut scratch);
                enc.decode_line(f).trim().parse::<f64>().ok()
            });
            match map.get_mut(&key) {
                Some(acc) => acc.add(value),
                None => {
                    map_bytes += key.len() + std::mem::size_of::<Acc>() + 48;
                    let mut acc = Acc::new();
                    acc.add(value);
                    map.insert(key, acc);
                    if map_bytes >= opts.budget_bytes {
                        spill_bytes += spill_group(&mut map, &opts.spill_dir, &mut runs)?;
                        map_bytes = 0;
                    }
                }
            }
        }
        start += advanced;
    }

    let mut groups = 0u64;
    if runs.is_empty() {
        // Fast path: everything fit in budget, no disk used. Emit sorted.
        let mut entries: Vec<(Vec<u8>, Acc)> = map.into_iter().collect();
        entries.par_sort_unstable_by(|a, b| a.0.cmp(&b.0));
        for (key, acc) in entries {
            emit(&group_row(key, &acc));
            groups += 1;
        }
    } else {
        if !map.is_empty() {
            spill_bytes += spill_group(&mut map, &opts.spill_dir, &mut runs)?;
        }
        groups = merge_groups(&runs, &mut emit)?;
        for r in &runs {
            let _ = fs::remove_file(r);
        }
    }

    Ok(GroupStats {
        groups,
        runs: runs.len(),
        spill_bytes,
    })
}

fn group_row(key: Vec<u8>, acc: &Acc) -> GroupRow {
    GroupRow {
        key,
        count: acc.count,
        numeric_count: acc.ncount,
        sum: acc.sum,
        min: acc.min,
        max: acc.max,
    }
}

fn key_bytes(
    raw: &[u8],
    enc: Encoding,
    col: Option<usize>,
    spec: &FieldSpec,
    scratch: &mut Vec<u8>,
) -> Vec<u8> {
    let field = extract_field(raw, col, spec, scratch);
    enc.decode_line(field).into_bytes()
}

fn spill_group(
    map: &mut std::collections::HashMap<Vec<u8>, Acc>,
    dir: &Path,
    runs: &mut Vec<PathBuf>,
) -> Result<u64> {
    let mut entries: Vec<(Vec<u8>, Acc)> = map.drain().collect();
    entries.par_sort_unstable_by(|a, b| a.0.cmp(&b.0));
    let path = dir.join(format!("grp{:05}.bin", runs.len()));
    let mut w = BufWriter::new(File::create(&path)?);
    let mut bytes = 0u64;
    for (key, acc) in &entries {
        w.write_all(&(key.len() as u32).to_le_bytes())?;
        w.write_all(key)?;
        w.write_all(&acc.count.to_le_bytes())?;
        w.write_all(&acc.ncount.to_le_bytes())?;
        w.write_all(&acc.sum.to_le_bytes())?;
        w.write_all(&acc.min.to_le_bytes())?;
        w.write_all(&acc.max.to_le_bytes())?;
        bytes += 4 + key.len() as u64 + 40;
    }
    w.flush()?;
    runs.push(path);
    Ok(bytes)
}

struct GroupRunReader {
    r: BufReader<File>,
}

impl GroupRunReader {
    fn open(path: &PathBuf) -> Result<GroupRunReader> {
        Ok(GroupRunReader {
            r: BufReader::new(File::open(path)?),
        })
    }
    fn next_record(&mut self) -> Result<Option<(Vec<u8>, Acc)>> {
        let mut len_b = [0u8; 4];
        if !read_full(&mut self.r, &mut len_b)? {
            return Ok(None);
        }
        let mut key = vec![0u8; u32::from_le_bytes(len_b) as usize];
        self.r.read_exact(&mut key)?;
        let mut f = [0u8; 40];
        self.r.read_exact(&mut f)?;
        let acc = Acc {
            count: u64::from_le_bytes(f[0..8].try_into().unwrap()),
            ncount: u64::from_le_bytes(f[8..16].try_into().unwrap()),
            sum: f64::from_le_bytes(f[16..24].try_into().unwrap()),
            min: f64::from_le_bytes(f[24..32].try_into().unwrap()),
            max: f64::from_le_bytes(f[32..40].try_into().unwrap()),
        };
        Ok(Some((key, acc)))
    }
}

/// Heap element ordered so the smallest key pops first (min-heap by key).
struct GroupHeapItem {
    key: Vec<u8>,
    run: usize,
    acc: Acc,
}
impl PartialEq for GroupHeapItem {
    fn eq(&self, o: &Self) -> bool {
        self.cmp(o) == Ordering::Equal
    }
}
impl Eq for GroupHeapItem {}
impl PartialOrd for GroupHeapItem {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}
impl Ord for GroupHeapItem {
    fn cmp(&self, o: &Self) -> Ordering {
        // Reversed: smaller key is "greater" so the max-heap yields it first.
        o.key.cmp(&self.key).then_with(|| o.run.cmp(&self.run))
    }
}

fn merge_groups(runs: &[PathBuf], emit: &mut impl FnMut(&GroupRow)) -> Result<u64> {
    let mut readers: Vec<GroupRunReader> = runs
        .iter()
        .map(GroupRunReader::open)
        .collect::<Result<_>>()?;
    let mut heap: BinaryHeap<GroupHeapItem> = BinaryHeap::with_capacity(readers.len());
    for (i, rr) in readers.iter_mut().enumerate() {
        if let Some((key, acc)) = rr.next_record()? {
            heap.push(GroupHeapItem { key, run: i, acc });
        }
    }

    let mut groups = 0u64;
    while let Some(first) = heap.pop() {
        let key = first.key;
        let mut acc = first.acc;
        if let Some((k, a)) = readers[first.run].next_record()? {
            heap.push(GroupHeapItem {
                key: k,
                run: first.run,
                acc: a,
            });
        }
        // Fold in the same key wherever it appears across the other runs.
        while heap.peek().is_some_and(|t| t.key == key) {
            let it = heap.pop().unwrap();
            acc.combine(&it.acc);
            if let Some((k, a)) = readers[it.run].next_record()? {
                heap.push(GroupHeapItem {
                    key: k,
                    run: it.run,
                    acc: a,
                });
            }
        }
        emit(&group_row(key, &acc));
        groups += 1;
    }
    Ok(groups)
}

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
    let mut scratch = Vec::new();
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
            let key = comparable_key(
                raw,
                enc,
                opts.key_column,
                &opts.fields,
                opts.numeric,
                &mut scratch,
            );
            if opts.largest {
                if min_heap.len() < n {
                    min_heap.push(Reverse((key, ln)));
                } else if matches!(min_heap.peek(), Some(Reverse((mk, _))) if key > *mk) {
                    min_heap.pop();
                    min_heap.push(Reverse((key, ln)));
                }
            } else if max_heap.len() < n {
                max_heap.push((key, ln));
            } else if matches!(max_heap.peek(), Some((mk, _)) if key < *mk) {
                max_heap.pop();
                max_heap.push((key, ln));
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

// ===================== DISTINCT (HyperLogLog) ================================

/// Options for [`distinct`].
#[derive(Clone, Debug)]
pub struct DistinctOptions {
    pub key_column: Option<usize>,
    pub fields: FieldSpec,
    /// HLL precision `p` (registers = 2^p). Clamped to [4, 18]. 14 ≈ 0.8% error.
    pub precision: u32,
}

impl Default for DistinctOptions {
    fn default() -> Self {
        DistinctOptions {
            key_column: None,
            fields: FieldSpec::default(),
            precision: 14,
        }
    }
}

/// Approximate distinct-value count of a field.
#[derive(Clone, Copy, Debug)]
pub struct DistinctResult {
    pub estimate: u64,
    pub registers: usize,
    pub memory_bytes: usize,
}

/// HyperLogLog: estimate cardinality in fixed memory (2^p bytes), independent of
/// how many distinct values there are.
struct Hll {
    reg: Vec<u8>,
    p: u32,
}

impl Hll {
    fn new(p: u32) -> Hll {
        Hll {
            reg: vec![0u8; 1usize << p],
            p,
        }
    }
    fn add(&mut self, h: u64) {
        let idx = (h >> (64 - self.p)) as usize; // top p bits select the register
                                                 // Remaining bits shifted to the top; a guard bit bounds rho.
        let w = (h << self.p) | (1u64 << (self.p - 1));
        let rho = w.leading_zeros() as u8 + 1;
        if rho > self.reg[idx] {
            self.reg[idx] = rho;
        }
    }
    fn estimate(&self) -> f64 {
        let m = self.reg.len() as f64;
        let alpha = match self.reg.len() {
            16 => 0.673,
            32 => 0.697,
            64 => 0.709,
            _ => 0.7213 / (1.0 + 1.079 / m),
        };
        let sum: f64 = self.reg.iter().map(|&r| 2f64.powi(-(r as i32))).sum();
        let raw = alpha * m * m / sum;
        if raw <= 2.5 * m {
            // Small-range correction: linear counting over empty registers.
            let zeros = self.reg.iter().filter(|&&r| r == 0).count() as f64;
            if zeros > 0.0 {
                return m * (m / zeros).ln();
            }
        }
        raw // 64-bit hash => no large-range (2^32) correction needed
    }
}

/// Estimate the number of distinct values of the configured field.
pub fn distinct(doc: &Document, opts: &DistinctOptions) -> DistinctResult {
    use std::hash::{Hash, Hasher};
    let total = doc.line_count();
    let mut hll = Hll::new(opts.precision.clamp(4, 18));
    let mut scratch = Vec::new();
    const BATCH: u64 = 8192;
    let mut start = 0u64;
    while start < total {
        let batch = doc.raw_line_ranges(start, BATCH);
        if batch.is_empty() {
            break;
        }
        let advanced = batch.len() as u64;
        for (_ln, raw) in batch {
            // Distinctness is over the (unescaped) field bytes; identical bytes
            // hash identically, so no decode is needed here.
            let field = extract_field(raw, opts.key_column, &opts.fields, &mut scratch);
            let mut h = std::collections::hash_map::DefaultHasher::new();
            field.hash(&mut h);
            hll.add(h.finish());
        }
        start += advanced;
    }
    DistinctResult {
        estimate: hll.estimate().round() as u64,
        registers: hll.reg.len(),
        memory_bytes: hll.reg.len(),
    }
}

/// Read exactly `buf.len()` bytes; `Ok(false)` if EOF before any byte was read,
/// `Err` on a partial read (a truncated record is corruption).
fn read_full<R: Read>(r: &mut R, buf: &mut [u8]) -> Result<bool> {
    let mut filled = 0;
    while filled < buf.len() {
        match r.read(&mut buf[filled..]) {
            Ok(0) => {
                if filled == 0 {
                    return Ok(false);
                }
                return Err(Error::Search("truncated spill record".into()));
            }
            Ok(n) => filled += n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(Error::Io(e)),
        }
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*; // brings the module's `std::io::Write` into scope for write_all
    use crate::document::OpenOptions;

    fn doc_from(bytes: &[u8]) -> (tempfile::NamedTempFile, Document) {
        let mut f = tempfile::NamedTempFile::new().unwrap();
        f.write_all(bytes).unwrap();
        f.flush().unwrap();
        let doc = Document::open(f.path(), &OpenOptions::default()).unwrap();
        (f, doc)
    }

    fn sorted_lines(doc: &Document, res: &SortResult) -> Vec<String> {
        let mut rd = OrderingReader::open(&res.ordering_path).unwrap();
        let mut out = Vec::new();
        while let Some(ln) = rd.next_line().unwrap() {
            out.push(doc.line(ln).unwrap());
        }
        out
    }

    #[test]
    fn numeric_sort_with_tiny_budget_spills_and_orders() {
        // Values in descending order; many lines so a tiny budget forces runs.
        let mut data = Vec::new();
        for i in (0..5000u64).rev() {
            data.extend_from_slice(format!("{i},row{i}\n").as_bytes());
        }
        let spill = tempfile::tempdir().unwrap();
        let (_f, doc) = doc_from(&data);
        let opts = SortOptions {
            key_column: Some(1),
            numeric: true,
            budget_bytes: 8 * 1024, // tiny => many spilled runs
            spill_dir: spill.path().to_path_buf(),
            ..Default::default()
        };
        let res = sort(&doc, &opts).unwrap();
        assert!(
            res.runs > 1,
            "tiny budget should produce multiple runs, got {}",
            res.runs
        );
        assert_eq!(res.line_count, 5000);
        let lines = sorted_lines(&doc, &res);
        assert_eq!(lines.first().unwrap(), "0,row0");
        assert_eq!(lines.last().unwrap(), "4999,row4999");
        // Fully ascending by numeric key.
        for (i, l) in lines.iter().enumerate() {
            assert_eq!(l, &format!("{i},row{i}"));
        }
    }

    #[test]
    fn lexicographic_reverse_and_whole_line() {
        let data = b"banana\napple\ncherry\napple\n";
        let spill = tempfile::tempdir().unwrap();
        let (_f, doc) = doc_from(data);
        let opts = SortOptions {
            key_column: None,
            reverse: true,
            budget_bytes: 1 << 20,
            spill_dir: spill.path().to_path_buf(),
            ..Default::default()
        };
        let res = sort(&doc, &opts).unwrap();
        let lines = sorted_lines(&doc, &res);
        assert_eq!(lines, vec!["cherry", "banana", "apple", "apple"]);
    }

    #[test]
    fn group_by_counts_and_sums_with_spill() {
        // 3 distinct keys (a,b,c); value column to sum. Tiny budget => spill+merge.
        let mut data = Vec::new();
        for i in 0..3000u64 {
            let k = ["a", "b", "c"][(i % 3) as usize];
            data.extend_from_slice(format!("{k},{i}\n").as_bytes());
        }
        let spill = tempfile::tempdir().unwrap();
        let (_f, doc) = doc_from(&data);
        let opts = GroupOptions {
            key_column: Some(1),
            value_column: Some(2),
            budget_bytes: 256, // force spilling and a merge across runs
            spill_dir: spill.path().to_path_buf(),
            ..Default::default()
        };
        let mut rows = Vec::new();
        let stats = group(&doc, &opts, |r| {
            rows.push((String::from_utf8_lossy(&r.key).into_owned(), r.count, r.sum));
        })
        .unwrap();
        assert_eq!(stats.groups, 3);
        assert!(
            stats.runs > 1,
            "tiny budget should spill, got {} runs",
            stats.runs
        );
        // Ascending key order; each key has 1000 rows.
        assert_eq!(rows[0].0, "a");
        assert_eq!(rows[1].0, "b");
        assert_eq!(rows[2].0, "c");
        assert_eq!(rows.iter().map(|r| r.1).sum::<u64>(), 3000);
        // Sum of i for key "a" = i in {0,3,6,...,2997}.
        let want_a: f64 = (0..3000u64).filter(|i| i % 3 == 0).map(|i| i as f64).sum();
        assert_eq!(rows[0].2, want_a);
    }

    #[test]
    fn csv_field_handles_quotes_and_escapes() {
        let line = br#"a,"x,y ""z""",c"#; // field 2 = x,y "z"  (quoted delim + "" escape)
        let mut out = Vec::new();
        csv_nth_field(line, b',', b'"', 1, &mut out);
        assert_eq!(out, b"a");
        csv_nth_field(line, b',', b'"', 2, &mut out);
        assert_eq!(out, &b"x,y \"z\""[..]);
        csv_nth_field(line, b',', b'"', 3, &mut out);
        assert_eq!(out, b"c");
        csv_nth_field(line, b',', b'"', 4, &mut out); // out of range
        assert!(out.is_empty());
    }

    #[test]
    fn csv_group_respects_quoted_delimiters() {
        // Without CSV mode, the comma inside quotes would split the key wrongly.
        let data = b"\"a,b\",1\n\"a,b\",2\nc,3\n";
        let spill = tempfile::tempdir().unwrap();
        let (_f, doc) = doc_from(data);
        let opts = GroupOptions {
            key_column: Some(1),
            value_column: Some(2),
            fields: FieldSpec {
                delimiter: b',',
                quote: b'"',
                csv: true,
            },
            budget_bytes: 1 << 20,
            spill_dir: spill.path().to_path_buf(),
        };
        let mut rows = Vec::new();
        group(&doc, &opts, |r| {
            rows.push((String::from_utf8_lossy(&r.key).into_owned(), r.count))
        })
        .unwrap();
        // "a,b" is one key with 2 rows; "c" has 1.
        assert_eq!(rows, vec![("a,b".into(), 2), ("c".into(), 1)]);
    }

    #[test]
    fn top_n_largest_and_smallest() {
        // (i*7) % 1000 is a permutation of 0..1000 (7 coprime to 1000).
        let mut data = Vec::new();
        for i in 0..1000u64 {
            data.extend_from_slice(format!("{},x\n", (i * 7) % 1000).as_bytes());
        }
        let (_f, doc) = doc_from(&data);
        let val = |ln: u64| doc.line(ln).unwrap().split(',').next().unwrap().to_string();

        let top = top_n(
            &doc,
            &TopOptions {
                key_column: Some(1),
                numeric: true,
                largest: true,
                n: 3,
                ..Default::default()
            },
        );
        assert_eq!(
            top.iter().map(|&l| val(l)).collect::<Vec<_>>(),
            vec!["999", "998", "997"]
        );

        let bot = top_n(
            &doc,
            &TopOptions {
                key_column: Some(1),
                numeric: true,
                largest: false,
                n: 2,
                ..Default::default()
            },
        );
        assert_eq!(
            bot.iter().map(|&l| val(l)).collect::<Vec<_>>(),
            vec!["0", "1"]
        );
    }

    #[test]
    fn distinct_estimate_is_close() {
        // 50,000 rows over exactly 5,000 distinct keys.
        let mut data = Vec::new();
        for i in 0..50_000u64 {
            data.extend_from_slice(format!("key{},v\n", i % 5000).as_bytes());
        }
        let (_f, doc) = doc_from(&data);
        let res = distinct(
            &doc,
            &DistinctOptions {
                key_column: Some(1),
                ..Default::default()
            },
        );
        let err = (res.estimate as f64 - 5000.0).abs() / 5000.0;
        assert!(
            err < 0.05,
            "HLL estimate {} too far from 5000 (rel err {:.3})",
            res.estimate,
            err
        );
    }

    #[test]
    fn group_no_spill_fast_path() {
        let data = b"x,1\ny,2\nx,3\ny,4\nx,5\n";
        let spill = tempfile::tempdir().unwrap();
        let (_f, doc) = doc_from(data);
        let opts = GroupOptions {
            key_column: Some(1),
            value_column: Some(2),
            budget_bytes: 1 << 20,
            spill_dir: spill.path().to_path_buf(),
            ..Default::default()
        };
        let mut rows = Vec::new();
        let stats = group(&doc, &opts, |r| {
            rows.push((
                String::from_utf8_lossy(&r.key).into_owned(),
                r.count,
                r.sum,
                r.avg(),
            ))
        })
        .unwrap();
        assert_eq!(stats.runs, 0, "small input should not spill");
        assert_eq!(
            rows,
            vec![
                ("x".into(), 3, 9.0, Some(3.0)),
                ("y".into(), 2, 6.0, Some(3.0)),
            ]
        );
    }

    #[test]
    fn numeric_handles_negatives_and_floats() {
        let data = b"3.5\n-2\n10\n-100.25\n0\n";
        let spill = tempfile::tempdir().unwrap();
        let (_f, doc) = doc_from(data);
        let opts = SortOptions {
            key_column: None,
            numeric: true,
            budget_bytes: 1 << 20,
            spill_dir: spill.path().to_path_buf(),
            ..Default::default()
        };
        let res = sort(&doc, &opts).unwrap();
        let lines = sorted_lines(&doc, &res);
        assert_eq!(lines, vec!["-100.25", "-2", "0", "3.5", "10"]);
    }
}
