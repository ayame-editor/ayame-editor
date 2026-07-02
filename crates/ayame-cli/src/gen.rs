//! `ayame gen` — generate synthetic big-data text for benchmarking and demos.
//!
//! Deterministic (seeded LCG) so a given `--lines` always produces identical
//! bytes, and streamed through a large `BufWriter` so producing tens of
//! gigabytes stays I/O-bound rather than allocation-bound.

use std::fs::File;
use std::io::{BufWriter, Write};
use std::time::Instant;

use anyhow::{Context, Result};
use ayame_core::Encoding;

use crate::{commas, first_opt, has_flag, human_bytes, parse_checked};

const WORDS: [&str; 16] = [
    "alpha", "bravo", "charlie", "delta", "echo", "foxtrot", "golf", "hotel", "india", "juliet",
    "kilo", "lima", "mike", "november", "oscar", "papa",
];
const JP_WORDS: [&str; 10] = [
    "東京",
    "大阪",
    "名古屋",
    "札幌",
    "福岡",
    "成功",
    "失敗",
    "再試行",
    "出荷",
    "注文",
];
const STATUS: [&str; 3] = ["ok", "warn", "error"];

#[inline]
fn lcg(state: &mut u64) -> u64 {
    *state = state
        .wrapping_mul(6364136223846793005)
        .wrapping_add(1442695040888963407);
    *state
}

pub fn cmd_gen(args: &[String]) -> Result<()> {
    let (pos, opts, flags) = parse_checked(
        args,
        &["--lines", "-n", "--cols", "--encoding"],
        &["--quiet", "-q"],
    )?;
    let path = pos.first().context("expected an output FILE")?;
    let lines: u64 = first_opt(&opts, &["--lines", "-n"])
        .context("--lines N is required")?
        .parse()
        .context("--lines must be a number")?;
    let cols: usize = first_opt(&opts, &["--cols"])
        .unwrap_or("5")
        .parse()
        .context("--cols must be a number")?;
    let enc = match first_opt(&opts, &["--encoding"]) {
        Some(e) => Encoding::parse(e).with_context(|| format!("unknown encoding '{e}'"))?,
        None => Encoding::Utf8,
    };
    let quiet = has_flag(&flags, &["--quiet", "-q"]);
    let japanese = matches!(enc, Encoding::ShiftJis | Encoding::EucJp);

    let file = File::create(path).with_context(|| format!("creating '{path}'"))?;
    let mut w = BufWriter::with_capacity(1 << 20, file);
    let t0 = Instant::now();
    let mut state: u64 = 0x9E3779B97F4A7C15;
    let mut line = String::with_capacity(128);

    for i in 0..lines {
        line.clear();
        // col0: id
        line.push_str(&i.to_string());
        // col1: pseudo timestamp
        let r = lcg(&mut state);
        line.push(',');
        line.push_str(&format!(
            "2026-06-30T{:02}:{:02}:{:02}.{:03}",
            (i / 3600) % 24,
            (i / 60) % 60,
            i % 60,
            r % 1000
        ));
        // col2: a word (Japanese for CJK encodings, to exercise the codec)
        line.push(',');
        if japanese {
            line.push_str(JP_WORDS[(lcg(&mut state) as usize) % JP_WORDS.len()]);
        } else {
            line.push_str(WORDS[(lcg(&mut state) as usize) % WORDS.len()]);
        }
        // col3: status
        line.push(',');
        line.push_str(STATUS[(lcg(&mut state) as usize) % STATUS.len()]);
        // col4..: numeric values
        for _ in 4..cols.max(5) {
            line.push(',');
            line.push_str(&(lcg(&mut state) % 1_000_000).to_string());
        }
        line.push('\n');

        match enc {
            Encoding::Utf8 | Encoding::Ascii => w.write_all(line.as_bytes())?,
            _ => {
                let bytes = enc
                    .encode_query(&line)
                    .context("line not representable in target encoding")?;
                w.write_all(&bytes)?;
            }
        }

        if !quiet && i > 0 && i % 10_000_000 == 0 {
            eprintln!("  … {} lines", commas(i));
        }
    }
    w.flush()?;

    if !quiet {
        let bytes = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
        let secs = t0.elapsed().as_secs_f64();
        eprintln!(
            "generated {} lines, {} in {:.2}s ({}/s)",
            commas(lines),
            human_bytes(bytes),
            secs,
            human_bytes((bytes as f64 / secs.max(1e-9)) as u64)
        );
    }
    Ok(())
}
