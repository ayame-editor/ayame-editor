use std::fs::{self, File};
use std::io::{BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};

use crate::document::Document;
use crate::fields::{comparable_key, FieldSpec};
use crate::Result;

use super::common::{read_full, unique_spill_dir};
use super::spill::{self, HeapEntry, Payload, RunReader};

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
            budget_bytes: super::common::DEFAULT_BUDGET_BYTES,
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
    sort_inner::<fn(u64, u64)>(doc, opts, None)
}

/// Sort with a coarse line-progress callback. The callback receives
/// `(processed_lines, total_lines)` after each scan batch.
pub fn sort_with_progress(
    doc: &Document,
    opts: &SortOptions,
    mut progress: impl FnMut(u64, u64),
) -> Result<SortResult> {
    sort_inner(doc, opts, Some(&mut progress))
}

fn sort_inner<F>(
    doc: &Document,
    opts: &SortOptions,
    mut progress: Option<&mut F>,
) -> Result<SortResult>
where
    F: FnMut(u64, u64),
{
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

    let mut scratch = Vec::new();
    report_progress(&mut progress, 0, total);
    doc.try_for_each_raw_line(
        |line_no, raw| {
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
            Ok(())
        },
        |done| report_progress(&mut progress, done, total),
    )?;
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

fn report_progress<F>(progress: &mut Option<&mut F>, done: u64, total: u64)
where
    F: FnMut(u64, u64),
{
    if let Some(cb) = progress.as_deref_mut() {
        cb(done.min(total), total);
    }
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

/// Build the merge-heap element for a sort run: the payload is the line number,
/// and the stable tie-break is that same line number (smaller emitted first),
/// so equal keys keep their original order across the merge.
fn heap_item(key: Vec<u8>, line_no: u64, run: usize, reverse: bool) -> HeapEntry<u64> {
    HeapEntry {
        key,
        payload: line_no,
        tiebreak: line_no,
        run,
        reverse,
    }
}

fn spill_run(
    records: &mut Vec<(Vec<u8>, u64)>,
    reverse: bool,
    dir: &Path,
    runs: &mut Vec<PathBuf>,
) -> Result<u64> {
    let path = dir.join(format!("run{:05}.bin", runs.len()));
    // Sort the run in the same direction the merge will consume it; ties break
    // on line number so the sort is stable.
    let bytes = spill::write_run(
        records,
        move |a, b| {
            let k = a.0.cmp(&b.0);
            let k = if reverse { k.reverse() } else { k };
            k.then_with(|| a.1.cmp(&b.1))
        },
        &path,
    )?;
    runs.push(path);
    Ok(bytes)
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

/// Intermediate merge pass: k-way-merge `runs` into one sorted run of full
/// `[len][key][line]` records, readable again by [`RunReader`].
fn merge_run_records(runs: &[PathBuf], reverse: bool, output_path: &Path) -> Result<(u64, u64)> {
    let mut readers: Vec<RunReader<u64>> = runs
        .iter()
        .map(|p| RunReader::open(p))
        .collect::<Result<_>>()?;
    let mut heap = spill::seed_heap(&mut readers, |key, line, run| {
        heap_item(key, line, run, reverse)
    })?;

    let mut out = BufWriter::new(File::create(output_path)?);
    let mut count = 0u64;
    let mut bytes = 0u64;
    while let Some(item) = heap.pop() {
        out.write_all(&(item.key.len() as u32).to_le_bytes())?;
        out.write_all(&item.key)?;
        item.payload.write_to(&mut out)?;
        count += 1;
        bytes += 4 + item.key.len() as u64 + 8;
        if let Some((key, line)) = readers[item.run].next_record()? {
            heap.push(heap_item(key, line, item.run, reverse));
        }
    }
    out.flush()?;
    Ok((count, bytes))
}

/// Final merge pass: k-way-merge `runs` and write only the sorted line numbers
/// (the ordering file), with no key framing.
fn merge_runs_to_ordering(runs: &[PathBuf], reverse: bool, ordering_path: &Path) -> Result<u64> {
    let mut readers: Vec<RunReader<u64>> = runs
        .iter()
        .map(|p| RunReader::open(p))
        .collect::<Result<_>>()?;
    let mut heap = spill::seed_heap(&mut readers, |key, line, run| {
        heap_item(key, line, run, reverse)
    })?;

    let mut out = BufWriter::new(File::create(ordering_path)?);
    let mut count = 0u64;
    while let Some(item) = heap.pop() {
        item.payload.write_to(&mut out)?;
        count += 1;
        if let Some((key, line)) = readers[item.run].next_record()? {
            heap.push(heap_item(key, line, item.run, reverse));
        }
    }
    out.flush()?;
    Ok(count)
}
