//! Generic external-merge (spill-to-disk) engine shared by `sort` and `group`.
//!
//! Both ops follow the same skeleton: spill sorted *runs* of `(key, payload)`
//! records to disk when an in-memory budget is exceeded, then k-way-merge the
//! runs with a bounded heap. They differ only in the payload they carry and in
//! what the merge does with equal keys (sort keeps every record; group folds
//! equal keys into one aggregate). This module factors out the shared machinery:
//! the on-disk run format, the run reader, the heap element, and one merge core.
//! A [`RunCodec`] implementation supplies the record type, its on-disk encoding,
//! and the heap ordering; a per-merge *combine* closure decides folding.

use std::cmp::Ordering;
use std::collections::BinaryHeap;
use std::fs::File;
use std::io::{BufReader, Read, Write};
use std::path::Path;

use crate::Result;

use super::common::read_full;

/// Shared default soft memory budget (bytes) for run generation before spilling,
/// used by both `SortOptions` and `GroupOptions`.
pub(super) const DEFAULT_BUDGET_BYTES: usize = 256 * 1024 * 1024;

/// Describes one kind of spilled record: its in-memory type, its on-disk
/// encoding, and how two records order in the merge heap.
///
/// The on-disk format is entirely owned by [`RunCodec::write`]/[`RunCodec::read`]
/// so callers never touch raw bytes; the heap ordering is owned by
/// [`RunCodec::compare`], which receives the per-merge [`RunCodec::Order`]
/// configuration (e.g. sort direction) so a single record type can be merged
/// ascending or descending.
pub(super) trait RunCodec {
    /// A single spilled record (key plus payload).
    type Record;
    /// Per-merge comparison configuration carried in each heap item (`Copy`),
    /// e.g. the sort direction. Use `()` when the ordering is fixed.
    type Order: Copy;

    /// Emission order of two records: `Less` means `a` is emitted before `b`.
    /// `a_run`/`b_run` are the source run indices, available as a stable
    /// tie-breaker. The merge uses this to pop records in emission order.
    fn compare(
        order: Self::Order,
        a: &Self::Record,
        a_run: usize,
        b: &Self::Record,
        b_run: usize,
    ) -> Ordering;

    /// Serialize one record to `w`, returning the number of bytes written.
    fn write<W: Write>(w: &mut W, rec: &Self::Record) -> Result<u64>;

    /// Deserialize one record, or `None` at a clean end of file.
    fn read<R: Read>(r: &mut R) -> Result<Option<Self::Record>>;
}

/// Sort `records` with `cmp`, then write them to `path` in `C`'s on-disk format.
/// Returns the number of bytes written. The caller supplies the comparator so
/// each op keeps its own in-run ordering (sort applies direction + a stable
/// line tie-break; group sorts by key ascending).
pub(super) fn write_run<C: RunCodec>(records: &[C::Record], path: &Path) -> Result<u64> {
    let mut w = std::io::BufWriter::new(File::create(path)?);
    let mut bytes = 0u64;
    for rec in records {
        bytes += C::write(&mut w, rec)?;
    }
    w.flush()?;
    Ok(bytes)
}

/// Streaming reader over one spilled run.
pub(super) struct RunReader {
    r: BufReader<File>,
}

impl RunReader {
    pub(super) fn open(path: &Path) -> Result<RunReader> {
        Ok(RunReader {
            r: BufReader::new(File::open(path)?),
        })
    }

    /// Read the next record using codec `C`, or `None` at end of file.
    pub(super) fn next<C: RunCodec>(&mut self) -> Result<Option<C::Record>> {
        C::read(&mut self.r)
    }
}

/// Heap element. Its `Ord` is arranged so the max-heap `BinaryHeap` pops
/// whichever record should be emitted next (the reverse of `C::compare`'s
/// emission order).
struct HeapItem<C: RunCodec> {
    record: C::Record,
    run: usize,
    order: C::Order,
}

impl<C: RunCodec> HeapItem<C> {
    fn ordering(&self, o: &Self) -> Ordering {
        // `compare` returns emission order (Less = emit first). The max-heap
        // pops its greatest element, so reverse it to pop the emit-first record.
        C::compare(self.order, &self.record, self.run, &o.record, o.run).reverse()
    }
}

impl<C: RunCodec> PartialEq for HeapItem<C> {
    fn eq(&self, o: &Self) -> bool {
        self.ordering(o) == Ordering::Equal
    }
}
impl<C: RunCodec> Eq for HeapItem<C> {}
impl<C: RunCodec> PartialOrd for HeapItem<C> {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}
impl<C: RunCodec> Ord for HeapItem<C> {
    fn cmp(&self, o: &Self) -> Ordering {
        self.ordering(o)
    }
}

/// K-way merge of sorted `runs`, emitting output records in `C::compare` order.
///
/// For each output position the smallest record is popped; then `combine` is
/// offered each subsequent record that is next in order. If `combine(acc, cand)`
/// returns `true`, `cand` has been folded into `acc` and is consumed (group's
/// equal-key aggregation); returning `false` stops folding and the accumulated
/// record is emitted as-is (sort never folds, so its `combine` always returns
/// `false`, emitting every record individually). `emit` receives each output
/// record by value and returns the bytes it wrote (0 if it does not spill).
///
/// Returns `(records_emitted, bytes_written)`.
pub(super) fn kway_merge<C, Cmb, Emit>(
    runs: &[&Path],
    order: C::Order,
    mut combine: Cmb,
    mut emit: Emit,
) -> Result<(u64, u64)>
where
    C: RunCodec,
    Cmb: FnMut(&mut C::Record, &C::Record) -> bool,
    Emit: FnMut(C::Record) -> Result<u64>,
{
    let mut readers: Vec<RunReader> = runs
        .iter()
        .map(|p| RunReader::open(p))
        .collect::<Result<_>>()?;

    let mut heap: BinaryHeap<HeapItem<C>> = BinaryHeap::with_capacity(readers.len());
    for (i, rr) in readers.iter_mut().enumerate() {
        if let Some(record) = rr.next::<C>()? {
            heap.push(HeapItem {
                record,
                run: i,
                order,
            });
        }
    }

    let mut count = 0u64;
    let mut bytes = 0u64;
    while let Some(first) = heap.pop() {
        let run0 = first.run;
        let mut acc = first.record;
        pull_next::<C>(&mut readers, &mut heap, run0, order)?;

        // Fold in subsequent records the combine policy accepts (equal keys).
        while let Some(top) = heap.peek() {
            if !combine(&mut acc, &top.record) {
                break;
            }
            let it = heap.pop().unwrap();
            pull_next::<C>(&mut readers, &mut heap, it.run, order)?;
        }

        bytes += emit(acc)?;
        count += 1;
    }
    Ok((count, bytes))
}

/// Advance run `run`'s reader and, if it yielded a record, push it on the heap.
fn pull_next<C: RunCodec>(
    readers: &mut [RunReader],
    heap: &mut BinaryHeap<HeapItem<C>>,
    run: usize,
    order: C::Order,
) -> Result<()> {
    if let Some(record) = readers[run].next::<C>()? {
        heap.push(HeapItem { record, run, order });
    }
    Ok(())
}

/// Read a length-prefixed key: `[len: u32 LE][len bytes]`. `None` at clean EOF.
/// Shared by codecs so the key framing stays identical on disk.
pub(super) fn read_key<R: Read>(r: &mut R) -> Result<Option<Vec<u8>>> {
    let mut len_b = [0u8; 4];
    if !read_full(r, &mut len_b)? {
        return Ok(None);
    }
    let mut key = vec![0u8; u32::from_le_bytes(len_b) as usize];
    r.read_exact(&mut key)?;
    Ok(Some(key))
}

/// Write a length-prefixed key, returning the bytes written (`4 + key.len()`).
pub(super) fn write_key<W: Write>(w: &mut W, key: &[u8]) -> Result<u64> {
    w.write_all(&(key.len() as u32).to_le_bytes())?;
    w.write_all(key)?;
    Ok(4 + key.len() as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Minimal codec: record is `(key, value)`, ordered by key ascending with a
    /// run tie-break, encoded as `[len][key][value: u64]`.
    struct TestCodec;
    impl RunCodec for TestCodec {
        type Record = (Vec<u8>, u64);
        type Order = ();
        fn compare(_: (), a: &Self::Record, ar: usize, b: &Self::Record, br: usize) -> Ordering {
            a.0.cmp(&b.0).then_with(|| ar.cmp(&br))
        }
        fn write<W: Write>(w: &mut W, rec: &Self::Record) -> Result<u64> {
            let n = write_key(w, &rec.0)?;
            w.write_all(&rec.1.to_le_bytes())?;
            Ok(n + 8)
        }
        fn read<R: Read>(r: &mut R) -> Result<Option<Self::Record>> {
            let Some(key) = read_key(r)? else {
                return Ok(None);
            };
            let mut v = [0u8; 8];
            r.read_exact(&mut v)?;
            Ok(Some((key, u64::from_le_bytes(v))))
        }
    }

    #[test]
    fn write_run_then_read_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("run.bin");
        let recs = vec![
            (b"a".to_vec(), 1u64),
            (b"bb".to_vec(), 2),
            (b"".to_vec(), 3),
        ];
        let bytes = write_run::<TestCodec>(&recs, &path).unwrap();
        // 3 records: (4+1+8) + (4+2+8) + (4+0+8) = 13 + 14 + 12 = 39.
        assert_eq!(bytes, 39);
        let mut rr = RunReader::open(&path).unwrap();
        let mut got = Vec::new();
        while let Some(rec) = rr.next::<TestCodec>().unwrap() {
            got.push(rec);
        }
        assert_eq!(got, recs);
    }

    #[test]
    fn kway_merge_orders_across_runs_without_folding() {
        let dir = tempfile::tempdir().unwrap();
        // Two sorted runs; merge must interleave into globally ascending key order.
        let p0 = dir.path().join("r0.bin");
        let p1 = dir.path().join("r1.bin");
        write_run::<TestCodec>(&[(b"a".to_vec(), 0), (b"c".to_vec(), 0)], &p0).unwrap();
        write_run::<TestCodec>(&[(b"b".to_vec(), 0), (b"d".to_vec(), 0)], &p1).unwrap();
        let mut keys = Vec::new();
        let (count, _) = kway_merge::<TestCodec, _, _>(
            &[p0.as_path(), p1.as_path()],
            (),
            |_, _| false, // no folding: emit every record
            |rec| {
                keys.push(String::from_utf8(rec.0).unwrap());
                Ok(0)
            },
        )
        .unwrap();
        assert_eq!(count, 4);
        assert_eq!(keys, ["a", "b", "c", "d"]);
    }

    #[test]
    fn kway_merge_folds_equal_keys() {
        let dir = tempfile::tempdir().unwrap();
        let p0 = dir.path().join("r0.bin");
        let p1 = dir.path().join("r1.bin");
        // "a" appears in both runs and should fold; values summed.
        write_run::<TestCodec>(&[(b"a".to_vec(), 1), (b"b".to_vec(), 10)], &p0).unwrap();
        write_run::<TestCodec>(&[(b"a".to_vec(), 2), (b"c".to_vec(), 20)], &p1).unwrap();
        let mut out = Vec::new();
        let (count, _) = kway_merge::<TestCodec, _, _>(
            &[p0.as_path(), p1.as_path()],
            (),
            |acc, cand| {
                if acc.0 == cand.0 {
                    acc.1 += cand.1;
                    true
                } else {
                    false
                }
            },
            |rec| {
                out.push((String::from_utf8(rec.0).unwrap(), rec.1));
                Ok(0)
            },
        )
        .unwrap();
        assert_eq!(count, 3);
        assert_eq!(
            out,
            [("a".into(), 3u64), ("b".into(), 10), ("c".into(), 20)]
        );
    }
}
