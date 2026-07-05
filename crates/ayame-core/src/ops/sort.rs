use std::cmp::Ordering;
use std::collections::BinaryHeap;
use std::fs::{self, File};
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};

use rayon::prelude::*;

use crate::document::Document;
use crate::fields::{comparable_key, FieldSpec};
use crate::Result;

use super::common::{read_full, unique_spill_dir};

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
    /// Number of initial spilled sorted runs generated before any merge pass.
    pub runs: usize,
    /// Total bytes written to spill runs (a proxy for disk used).
    pub spill_bytes: u64,
}

/// Sort `doc` by the configured key, returning an ordering file.
///
/// Phase 1 streams the document in line-aligned batches, building comparable
/// byte keys, and spills sorted `(key, line_no)` runs whenever the in-memory
/// buffer reaches `budget_bytes`. Phase 2 k-way-merges the runs with a bounded
/// fan-in heap, writing the sorted line numbers. Peak memory is
/// `O(budget + fan-in)`, not `O(number_of_runs)`.
pub fn sort(doc: &Document, opts: &SortOptions) -> Result<SortResult> {
    let spill_dir = unique_spill_dir(&opts.spill_dir)?;
    let run_name = spill_dir
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("run");
    let ordering_path = opts.spill_dir.join(format!("{run_name}.ordering.bin"));

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
            let key = comparable_key(
                raw,
                enc,
                opts.key_column,
                &opts.fields,
                opts.numeric,
                &mut scratch,
            );
            buffered_bytes += key.len() + 40; // key + Vec/tuple overhead estimate
            buffer.push((key, line_no));
            if buffered_bytes >= opts.budget_bytes {
                spill_bytes += spill_run(&mut buffer, opts.reverse, &spill_dir, &mut runs)?;
                buffered_bytes = 0;
            }
        }
        start += advanced;
    }
    if !buffer.is_empty() {
        spill_bytes += spill_run(&mut buffer, opts.reverse, &spill_dir, &mut runs)?;
    }

    // ---- phase 2: k-way merge -------------------------------------------
    let ordering_tmp = spill_dir.join("ordering.bin");
    let (merged, merge_spill_bytes) = merge_runs(&runs, opts.reverse, &ordering_tmp)?;
    spill_bytes += merge_spill_bytes;
    // Runs are consumed; delete them so only the ordering remains.
    for r in &runs {
        let _ = fs::remove_file(r);
    }
    fs::rename(&ordering_tmp, &ordering_path)?;
    let _ = fs::remove_dir(&spill_dir);

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

// ---- run generation / merge --------------------------------------------------

pub(super) const MERGE_FAN_IN: usize = 64;

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

fn merge_runs(runs: &[PathBuf], reverse: bool, ordering_path: &Path) -> Result<(u64, u64)> {
    if runs.is_empty() {
        BufWriter::new(File::create(ordering_path)?).flush()?;
        return Ok((0, 0));
    }

    let mut current = runs.to_vec();
    let mut pass = 0usize;
    let mut extra_spill_bytes = 0u64;
    while current.len() > MERGE_FAN_IN {
        pass += 1;
        let mut next = Vec::new();
        for (chunk_idx, chunk) in current.chunks(MERGE_FAN_IN).enumerate() {
            let path = intermediate_run_path(ordering_path, pass, chunk_idx);
            let (_count, bytes) = merge_run_records(chunk, reverse, &path)?;
            extra_spill_bytes += bytes;
            next.push(path);
        }
        for p in &current {
            let _ = fs::remove_file(p);
        }
        current = next;
    }

    let count = merge_runs_to_ordering(&current, reverse, ordering_path)?;
    for p in &current {
        let _ = fs::remove_file(p);
    }
    Ok((count, extra_spill_bytes))
}

fn intermediate_run_path(ordering_path: &Path, pass: usize, chunk_idx: usize) -> PathBuf {
    let dir = ordering_path.parent().unwrap_or_else(|| Path::new("."));
    dir.join(format!("merge-pass{pass:03}-chunk{chunk_idx:05}.bin"))
}

fn merge_run_records(runs: &[PathBuf], reverse: bool, output_path: &Path) -> Result<(u64, u64)> {
    let mut readers: Vec<RunReader> = runs
        .iter()
        .map(|p| RunReader::open(p))
        .collect::<Result<_>>()?;
    let mut heap = seed_merge_heap(&mut readers, reverse)?;

    let mut out = BufWriter::new(File::create(output_path)?);
    let mut count = 0u64;
    let mut bytes = 0u64;
    while let Some(item) = heap.pop() {
        let key_len = item.key.len() as u32;
        out.write_all(&key_len.to_le_bytes())?;
        out.write_all(&item.key)?;
        out.write_all(&item.line_no.to_le_bytes())?;
        count += 1;
        bytes += 4 + item.key.len() as u64 + 8;
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
    Ok((count, bytes))
}

fn merge_runs_to_ordering(runs: &[PathBuf], reverse: bool, ordering_path: &Path) -> Result<u64> {
    let mut readers: Vec<RunReader> = runs
        .iter()
        .map(|p| RunReader::open(p))
        .collect::<Result<_>>()?;
    let mut heap = seed_merge_heap(&mut readers, reverse)?;

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

fn seed_merge_heap(readers: &mut [RunReader], reverse: bool) -> Result<BinaryHeap<HeapItem>> {
    let mut heap = BinaryHeap::with_capacity(readers.len());
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
    Ok(heap)
}
