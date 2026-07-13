use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use rayon::prelude::*;

use crate::document::Document;
use crate::fields::{decoded_text_key_into, field_bytes, FieldSpec};
use crate::Result;

use super::common::{unique_spill_dir, SpillCleanup};
use super::spill::{self, HeapEntry, Payload, RunReader};

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
            budget_bytes: super::common::DEFAULT_BUDGET_BYTES,
            spill_dir: std::env::temp_dir().join("ayame-group"),
        }
    }
}

/// One aggregated group emitted by [`group`].
#[derive(Clone, Debug)]
pub struct GroupRow {
    pub key: Vec<u8>,
    pub count: u64,
    /// How many rows had a parseable finite numeric value (for sum/min/max/avg).
    pub numeric_count: u64,
    /// Correctly-rounded sum of the group's finite values (0.0 for none).
    pub sum: f64,
    /// Smallest finite value, or `None` when the group had no numeric values —
    /// previously this leaked the internal `+inf` sentinel (#197).
    pub min: Option<f64>,
    /// Largest finite value, or `None` when the group had no numeric values.
    pub max: Option<f64>,
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

/// Exact f64 accumulator: Shewchuk's nonoverlapping-expansion summation, the
/// algorithm behind Python's `math.fsum`. The expansion `partials` represents
/// the running sum *exactly*, so the final [`ExactSum::value`] is the
/// correctly-rounded sum of the inputs — a function of the input multiset
/// only, independent of accumulation order. That is what makes group-by
/// aggregates deterministic across spill boundaries: adding values one by one
/// (in-memory path) and combining per-run partial sums (spill path) reach the
/// same exact value, where plain `f64 +=` differs because float addition is
/// not associative (#197).
#[derive(Clone, Copy, Debug)]
struct ExactSum {
    /// Nonoverlapping partials in increasing magnitude; their exact sum is
    /// the accumulated value. Finite doubles allow at most ~40 nonoverlapping
    /// terms (the exponent range over the 53-bit mantissa width).
    partials: [f64; Self::MAX],
    len: u8,
}

impl ExactSum {
    const MAX: usize = 40;

    fn new() -> ExactSum {
        ExactSum {
            partials: [0.0; Self::MAX],
            len: 0,
        }
    }

    fn add(&mut self, mut x: f64) {
        let mut i = 0usize;
        for j in 0..self.len as usize {
            let mut y = self.partials[j];
            if x.abs() < y.abs() {
                std::mem::swap(&mut x, &mut y);
            }
            let hi = x + y;
            let lo = y - (hi - x);
            if lo != 0.0 {
                self.partials[i] = lo;
                i += 1;
            }
            x = hi;
        }
        if !x.is_finite() {
            // The exact sum left the finite f64 range (callers filter the
            // inputs, so this needs values summing past ~1.8e308). Collapse to
            // a single saturated term; further adds keep it saturated.
            self.partials[0] = x;
            self.len = 1;
            return;
        }
        // `i` cannot reach MAX while the nonoverlapping invariant holds; fold
        // defensively rather than index out of bounds if it ever breaks.
        let i = i.min(Self::MAX - 1);
        self.partials[i] = x;
        self.len = (i + 1) as u8;
    }

    fn combine(&mut self, other: &ExactSum) {
        for j in 0..other.len as usize {
            self.add(other.partials[j]);
        }
    }

    /// Correctly-rounded (round-half-even) value of the exact sum. Ported
    /// from CPython's `math.fsum` finalization, including its halfway-case
    /// correction against the next partial.
    fn value(&self) -> f64 {
        let p = &self.partials[..self.len as usize];
        let mut n = p.len();
        if n == 0 {
            return 0.0;
        }
        n -= 1;
        let mut hi = p[n];
        let mut lo = 0.0;
        while n > 0 {
            let x = hi;
            n -= 1;
            let y = p[n];
            hi = x + y;
            let yr = hi - x;
            lo = y - yr;
            if lo != 0.0 {
                break;
            }
        }
        if n > 0 && ((lo < 0.0 && p[n - 1] < 0.0) || (lo > 0.0 && p[n - 1] > 0.0)) {
            let y = lo * 2.0;
            let x = hi + y;
            if y == x - hi {
                hi = x;
            }
        }
        hi
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
    sum: ExactSum,
    /// Internal sentinels (`+inf`/`-inf` when no numeric value was seen);
    /// [`group_row`] translates them to `None` instead of leaking them (#197).
    min: f64,
    max: f64,
}

impl Acc {
    fn new() -> Acc {
        Acc {
            count: 0,
            ncount: 0,
            sum: ExactSum::new(),
            min: f64::INFINITY,
            max: f64::NEG_INFINITY,
        }
    }
    fn add(&mut self, v: Option<f64>) {
        self.count += 1;
        if let Some(x) = v {
            self.ncount += 1;
            self.sum.add(x);
            self.min = self.min.min(x);
            self.max = self.max.max(x);
        }
    }
    fn combine(&mut self, o: &Acc) {
        self.count += o.count;
        self.ncount += o.ncount;
        self.sum.combine(&o.sum);
        self.min = self.min.min(o.min);
        self.max = self.max.max(o.max);
    }
}

impl Payload for Acc {
    // count + ncount + min + max + partials length + fixed partials block.
    const LEN: usize = 8 + 8 + 8 + 8 + 1 + ExactSum::MAX * 8;
    fn write_to(&self, out: &mut impl Write) -> std::io::Result<()> {
        out.write_all(&self.count.to_le_bytes())?;
        out.write_all(&self.ncount.to_le_bytes())?;
        out.write_all(&self.min.to_le_bytes())?;
        out.write_all(&self.max.to_le_bytes())?;
        out.write_all(&[self.sum.len])?;
        for p in &self.sum.partials {
            out.write_all(&p.to_le_bytes())?;
        }
        Ok(())
    }
    fn read_from(bytes: &[u8]) -> Acc {
        let mut sum = ExactSum::new();
        sum.len = bytes[32].min(ExactSum::MAX as u8);
        for (i, p) in sum.partials.iter_mut().enumerate() {
            let at = 33 + i * 8;
            *p = f64::from_le_bytes(bytes[at..at + 8].try_into().unwrap());
        }
        Acc {
            count: u64::from_le_bytes(bytes[0..8].try_into().unwrap()),
            ncount: u64::from_le_bytes(bytes[8..16].try_into().unwrap()),
            min: f64::from_le_bytes(bytes[16..24].try_into().unwrap()),
            max: f64::from_le_bytes(bytes[24..32].try_into().unwrap()),
            sum,
        }
    }
}

/// Group `doc` by the key field and aggregate, calling `emit` once per group in
/// ascending key order. Keeps an in-memory map up to `budget_bytes`; when it
/// overflows it spills a sorted partial-aggregate run and continues, then
/// k-way-merges the runs combining equal keys. The common case (few groups)
/// never touches disk; unbounded cardinality stays within budget.
///
/// Aggregates are deterministic: sums accumulate exactly (see [`ExactSum`]),
/// so `sum`/`avg` do not depend on `budget_bytes` or on how rows were split
/// across spill runs, and non-finite value strings (`NaN`, `inf`, `1e999`…)
/// are excluded from sum/min/max/avg — mirroring how sort treats them (#197).
pub fn group(
    doc: &Document,
    opts: &GroupOptions,
    mut emit: impl FnMut(&GroupRow),
) -> Result<GroupStats> {
    use std::collections::HashMap;
    let spill_dir = unique_spill_dir(&opts.spill_dir)?;
    // Any exit before the final disarm — error or panic (including one in the
    // caller's `emit`) — must leave no runs or spill directory behind (#201).
    let mut cleanup = SpillCleanup::new(spill_dir.clone());
    let enc = doc.encoding();

    let mut map: HashMap<Vec<u8>, Acc> = HashMap::new();
    let mut map_bytes = 0usize;
    let mut runs: Vec<PathBuf> = Vec::new();
    let mut spill_bytes = 0u64;

    let mut field_scratch = Vec::new();
    let mut key_scratch = Vec::new();
    // Logical records (RFC-4180 in CSV mode), so a quoted field containing a
    // newline keys as one record instead of two broken "lines" (#199).
    super::common::try_for_each_record(
        doc,
        &opts.fields,
        |_record_no, raw, _start, _raw_end| {
            decoded_text_key_into(
                raw,
                enc,
                opts.key_column,
                &opts.fields,
                &mut field_scratch,
                &mut key_scratch,
            );
            let value = opts.value_column.and_then(|c| {
                let f = field_bytes(raw, Some(c), &opts.fields, &mut field_scratch);
                // Rust's f64 parser accepts "NaN"/"inf"/"infinity", and huge
                // literals like "1e999" overflow to +inf — one such cell would
                // poison the whole group's sum/avg. Treat anything non-finite
                // as non-numeric, like the sort keys do (#197).
                enc.decode_line(f)
                    .trim()
                    .parse::<f64>()
                    .ok()
                    .filter(|v| v.is_finite())
            });
            match map.get_mut(key_scratch.as_slice()) {
                Some(acc) => acc.add(value),
                None => {
                    map_bytes += key_scratch.len() + std::mem::size_of::<Acc>() + 48;
                    let mut acc = Acc::new();
                    acc.add(value);
                    map.insert(key_scratch.clone(), acc);
                    if map_bytes >= opts.budget_bytes {
                        spill_bytes += spill_group(&mut map, &spill_dir, &mut runs)?;
                        map_bytes = 0;
                    }
                }
            }
            Ok(())
        },
        |_| {},
    )?;

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
            spill_bytes += spill_group(&mut map, &spill_dir, &mut runs)?;
        }
        groups = merge_groups(&runs, &mut emit)?;
        for r in &runs {
            let _ = fs::remove_file(r);
        }
    }
    let _ = fs::remove_dir(&spill_dir);
    cleanup.disarm();

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
        sum: acc.sum.value(),
        min: (acc.ncount > 0).then_some(acc.min),
        max: (acc.ncount > 0).then_some(acc.max),
    }
}

/// Build the merge-heap element for a group run: the payload is the aggregate,
/// and the tie-break is the run index. Equal keys are folded during the merge,
/// so their order among runs is irrelevant — it only has to be deterministic.
fn group_heap_item(key: Vec<u8>, acc: Acc, run: usize) -> HeapEntry<Acc> {
    HeapEntry {
        key,
        payload: acc,
        tiebreak: run as u64,
        run,
        reverse: false,
    }
}

fn spill_group(
    map: &mut std::collections::HashMap<Vec<u8>, Acc>,
    dir: &Path,
    runs: &mut Vec<PathBuf>,
) -> Result<u64> {
    let mut entries: Vec<(Vec<u8>, Acc)> = map.drain().collect();
    let path = dir.join(format!("grp{:05}.bin", runs.len()));
    let bytes = spill::write_run(&mut entries, |a, b| a.0.cmp(&b.0), &path)?;
    runs.push(path);
    Ok(bytes)
}

/// K-way-merge the spilled runs in a single pass, folding equal keys back
/// together — group runs hold partial aggregates, so any key appearing across
/// runs just needs its accumulators combined.
fn merge_groups(runs: &[PathBuf], emit: &mut impl FnMut(&GroupRow)) -> Result<u64> {
    let mut readers: Vec<RunReader<Acc>> = runs
        .iter()
        .map(|p| RunReader::open(p))
        .collect::<Result<_>>()?;
    let mut heap = spill::seed_heap(&mut readers, group_heap_item)?;

    let mut groups = 0u64;
    while let Some(first) = heap.pop() {
        let key = first.key;
        let mut acc = first.payload;
        if let Some((k, a)) = readers[first.run].next_record()? {
            heap.push(group_heap_item(k, a, first.run));
        }
        // Fold in the same key wherever it appears across the other runs.
        while heap.peek().is_some_and(|t| t.key == key) {
            let it = heap.pop().unwrap();
            acc.combine(&it.payload);
            if let Some((k, a)) = readers[it.run].next_record()? {
                heap.push(group_heap_item(k, a, it.run));
            }
        }
        emit(&group_row(key, &acc));
        groups += 1;
    }
    Ok(groups)
}
