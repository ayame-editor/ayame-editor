//! Out-of-core data operations over a [`Document`].
//!
//! These ops are bounded by an explicit **memory budget** and spill the
//! overflow to disk — the project's core trade-off (spend disk to bound memory
//! and stay stable). The first op is an external merge sort; group-by/top-n
//! follow the same shape.
//!
//! Sorting produces an **ordering**: a file of `u64` line numbers in sorted
//! order. The viewer can page through that ordering via the existing sparse
//! fetch path, so a sorted view of ten billion lines never materializes the
//! lines themselves — only their order.

use std::cmp::Ordering;
use std::collections::BinaryHeap;
use std::fs::{self, File};
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};

use rayon::prelude::*;

use crate::document::Document;
use crate::encoding::Encoding;
use crate::{Error, Result};

/// How to sort.
#[derive(Clone, Debug)]
pub struct SortOptions {
    /// 1-based field index to sort on; `None` sorts on the whole line.
    pub key_column: Option<usize>,
    /// Field delimiter used when `key_column` is set.
    pub delimiter: u8,
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
            delimiter: b',',
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
    while start < total {
        let batch = doc.raw_line_ranges(start, BATCH);
        if batch.is_empty() {
            break;
        }
        let advanced = batch.len() as u64;
        for (line_no, raw) in batch {
            let key = make_key(raw, enc, opts);
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
        Ok(OrderingReader { r: BufReader::new(File::open(path)?) })
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

fn make_key(raw: &[u8], enc: Encoding, opts: &SortOptions) -> Vec<u8> {
    let field = match opts.key_column {
        Some(col) => nth_field(raw, opts.delimiter, col),
        None => raw,
    };
    if opts.numeric {
        // Parse the decoded field; unparseable values sort last (ascending).
        let s = enc.decode_line(field);
        let v = s.trim().parse::<f64>().unwrap_or(f64::INFINITY);
        f64_order_key(v).to_vec()
    } else {
        // Decode to UTF-8: byte order of UTF-8 == Unicode code-point order, so a
        // Shift_JIS/EUC-JP file sorts in code-point order, not raw-byte order.
        enc.decode_line(field).into_bytes()
    }
}

/// 1-based field by delimiter, as a raw byte slice (empty if out of range).
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
        Ok(RunReader { r: BufReader::new(File::open(path)?) })
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
        let primary = if self.reverse { key_ord } else { key_ord.reverse() };
        // Tie-break: smaller line number emitted first => it must be "greater".
        primary.then_with(|| o.line_no.cmp(&self.line_no))
    }
}

fn merge_runs(runs: &[PathBuf], reverse: bool, ordering_path: &Path) -> Result<u64> {
    let mut readers: Vec<RunReader> =
        runs.iter().map(|p| RunReader::open(p)).collect::<Result<_>>()?;
    let mut heap: BinaryHeap<HeapItem> = BinaryHeap::with_capacity(readers.len());
    for (i, rr) in readers.iter_mut().enumerate() {
        if let Some((key, line_no)) = rr.next_record()? {
            heap.push(HeapItem { key, line_no, run: i, reverse });
        }
    }

    let mut out = BufWriter::new(File::create(ordering_path)?);
    let mut count = 0u64;
    while let Some(item) = heap.pop() {
        out.write_all(&item.line_no.to_le_bytes())?;
        count += 1;
        if let Some((key, line_no)) = readers[item.run].next_record()? {
            heap.push(HeapItem { key, line_no, run: item.run, reverse });
        }
    }
    out.flush()?;
    Ok(count)
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
        assert!(res.runs > 1, "tiny budget should produce multiple runs, got {}", res.runs);
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
