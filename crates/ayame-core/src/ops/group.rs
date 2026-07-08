use std::cmp::Ordering;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use rayon::prelude::*;

use crate::document::Document;
use crate::fields::{decoded_text_key_into, field_bytes, FieldSpec};
use crate::Result;

use super::common::unique_spill_dir;
use super::spill::{self, RunCodec};

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
            budget_bytes: spill::DEFAULT_BUDGET_BYTES,
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

/// Codec for a spilled group: on-disk `[len][key][count][ncount][sum][min][max]`
/// (five 8-byte LE fields), ordered by key ascending with a run tie-break so the
/// merge folds equal keys in a stable run order.
struct GroupCodec;

impl RunCodec for GroupCodec {
    type Record = (Vec<u8>, Acc);
    type Order = ();

    fn compare(
        _order: (),
        a: &(Vec<u8>, Acc),
        a_run: usize,
        b: &(Vec<u8>, Acc),
        b_run: usize,
    ) -> Ordering {
        a.0.cmp(&b.0).then_with(|| a_run.cmp(&b_run))
    }

    fn write<W: Write>(w: &mut W, rec: &(Vec<u8>, Acc)) -> Result<u64> {
        let (key, acc) = rec;
        let n = spill::write_key(w, key)?;
        w.write_all(&acc.count.to_le_bytes())?;
        w.write_all(&acc.ncount.to_le_bytes())?;
        w.write_all(&acc.sum.to_le_bytes())?;
        w.write_all(&acc.min.to_le_bytes())?;
        w.write_all(&acc.max.to_le_bytes())?;
        Ok(n + 40)
    }

    fn read<R: Read>(r: &mut R) -> Result<Option<(Vec<u8>, Acc)>> {
        let Some(key) = spill::read_key(r)? else {
            return Ok(None);
        };
        let mut f = [0u8; 40];
        r.read_exact(&mut f)?;
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
    let spill_dir = unique_spill_dir(&opts.spill_dir)?;
    let enc = doc.encoding();

    let mut map: HashMap<Vec<u8>, Acc> = HashMap::new();
    let mut map_bytes = 0usize;
    let mut runs: Vec<PathBuf> = Vec::new();
    let mut spill_bytes = 0u64;

    let mut field_scratch = Vec::new();
    let mut key_scratch = Vec::new();
    doc.for_each_raw_line(|_ln, raw| {
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
            enc.decode_line(f).trim().parse::<f64>().ok()
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
    })?;

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

fn spill_group(
    map: &mut std::collections::HashMap<Vec<u8>, Acc>,
    dir: &Path,
    runs: &mut Vec<PathBuf>,
) -> Result<u64> {
    let mut entries: Vec<(Vec<u8>, Acc)> = map.drain().collect();
    entries.par_sort_unstable_by(|a, b| a.0.cmp(&b.0));
    let path = dir.join(format!("grp{:05}.bin", runs.len()));
    let bytes = spill::write_run::<GroupCodec>(&entries, &path)?;
    runs.push(path);
    Ok(bytes)
}

fn merge_groups(runs: &[PathBuf], emit: &mut impl FnMut(&GroupRow)) -> Result<u64> {
    let run_refs: Vec<&Path> = runs.iter().map(|p| p.as_path()).collect();
    let (groups, _bytes) = spill::kway_merge::<GroupCodec, _, _>(
        &run_refs,
        (),
        |acc, cand| {
            // Fold `cand` into `acc` when the keys match (equal-key aggregation).
            if acc.0 == cand.0 {
                acc.1.combine(&cand.1);
                true
            } else {
                false
            }
        },
        |(key, acc)| {
            emit(&group_row(key, &acc));
            Ok(0)
        },
    )?;
    Ok(groups)
}
