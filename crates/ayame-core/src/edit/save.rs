//! Saving the overlay to disk, byte-exact or re-encoded.

use super::*;

impl EditSession {
    pub fn save_to_path(&self, doc: &Document, target: impl AsRef<Path>) -> Result<SaveResult> {
        self.save_to_path_inner(doc, target.as_ref(), false)
    }

    pub fn save_to_path_overwrite(
        &self,
        doc: &Document,
        target: impl AsRef<Path>,
    ) -> Result<SaveResult> {
        self.save_to_path_inner(doc, target.as_ref(), true)
    }

    /// Save the logical document (edits applied) to `target`, re-encoding every
    /// line to `enc` and terminating each with `eol`.
    ///
    /// Unlike [`Edits::save_to_path`], which copies untouched lines out of the
    /// mmap as raw bytes, this decodes and re-encodes every line — O(total
    /// bytes) — so it can change the file's 文字コード (encoding) and 改行コード
    /// (line ending). Whether the last line gets a terminator mirrors the
    /// source file. Fails if a line holds a character `enc` cannot represent,
    /// rather than writing a lossy file.
    ///
    /// When `with_bom` is set, a byte-order mark is written for Unicode
    /// encodings that define one (UTF-8 / UTF-16LE / UTF-16BE).
    pub fn save_converted(
        &self,
        doc: &Document,
        target: impl AsRef<Path>,
        enc: crate::Encoding,
        eol: crate::Eol,
        with_bom: bool,
        overwrite: bool,
    ) -> Result<SaveResult> {
        let target = target.as_ref();
        if target.exists() && !overwrite {
            return Err(Error::TargetExists {
                path: target.to_path_buf(),
            });
        }
        let tmp = temp_path(target);
        let file = OpenOptions::new().write(true).create_new(true).open(&tmp)?;
        let mut w = BufWriter::new(file);
        if with_bom {
            w.write_all(enc.bom())?;
        }
        let term = enc.encode_text(eol_text(eol)).ok_or_else(|| {
            Error::InvalidInput(format!(
                "{} line endings cannot be written as {}",
                eol.label(),
                enc.label()
            ))
        })?;
        let total = self.total_lines(doc);
        let ends_nl = document_ends_with_newline(doc);
        let original_total = doc.line_count();
        {
            let mut converted = ConvertedWriter {
                enc,
                terminator: &term,
                total,
                ends_nl,
                logical: 0,
                w: &mut w,
            };
            let mut next_original = 0u64;
            for (&anchor, ev) in self.events.range(..original_total) {
                let anchor = anchor.min(original_total);
                if anchor < next_original {
                    continue;
                }
                converted.write_original_span(doc, next_original, anchor)?;
                for text in &ev.inserts {
                    converted.write_text(text)?;
                }
                if anchor < original_total {
                    if ev.deleted {
                        next_original = anchor + 1;
                    } else {
                        if let Some(text) = &ev.replacement {
                            converted.write_text(text)?;
                        } else {
                            converted.write_original_span(doc, anchor, anchor + 1)?;
                        }
                        next_original = anchor + 1;
                    }
                }
            }
            converted.write_original_span(doc, next_original, original_total)?;
            if let Some(ev) = self.events.get(&original_total) {
                for text in &ev.inserts {
                    converted.write_text(text)?;
                }
            }
        }
        w.flush()?;
        w.get_ref().sync_all()?;
        drop(w);
        // The converted bytes were decoded from the mmap; if the base file
        // shrank mid-save they contain zero-fill — abort before publishing.
        if let Err(e) = doc.verify_base() {
            let _ = std::fs::remove_file(&tmp);
            return Err(e);
        }
        commit_temp_file(&tmp, target, overwrite)?;
        let bytes = std::fs::metadata(target)?.len();
        Ok(SaveResult {
            path: target.to_path_buf(),
            bytes,
            lines: total,
        })
    }

    fn save_to_path_inner(
        &self,
        doc: &Document,
        target: &Path,
        overwrite: bool,
    ) -> Result<SaveResult> {
        if target.exists() && !overwrite {
            return Err(Error::TargetExists {
                path: target.to_path_buf(),
            });
        }
        self.write_stream(doc, target, overwrite)?;
        let bytes = std::fs::metadata(target)?.len();
        Ok(SaveResult {
            path: target.to_path_buf(),
            bytes,
            lines: self.total_lines(doc),
        })
    }

    fn write_stream(&self, doc: &Document, target: &Path, overwrite: bool) -> Result<()> {
        let tmp = temp_path(target);
        let file = OpenOptions::new().write(true).create_new(true).open(&tmp)?;
        let mut w = BufWriter::new(file);

        w.write_all(doc.prefix_bytes())?;
        let mut output = OutputState::default();
        let original_total = doc.line_count();
        // Walk only the (sparse) edited anchors. Every run of untouched
        // original lines between two anchors is one contiguous mmap byte
        // range, copied out with a single write. This keeps saving
        // O(edits × stride + bytes) instead of the previous per-line random
        // access, which cost O(lines × stride).
        let mut next_unwritten = 0u64;
        for (&anchor, ev) in self.events.range(..original_total) {
            copy_original_span(&mut w, doc, next_unwritten, anchor, &mut output)?;
            next_unwritten = anchor;
            for text in &ev.inserts {
                write_edited_line(&mut w, doc, text, doc.default_terminator(), &mut output)?;
            }
            if ev.deleted {
                next_unwritten = anchor + 1;
                continue;
            }
            if let Some(text) = &ev.replacement {
                let term = doc.line_terminator(anchor).unwrap_or(b"");
                write_edited_line(&mut w, doc, text, term, &mut output)?;
                next_unwritten = anchor + 1;
            }
            // An event carrying only inserts leaves its anchor line untouched;
            // `next_unwritten` stays at `anchor` so the next contiguous copy
            // starts with that original line.
        }
        copy_original_span(&mut w, doc, next_unwritten, original_total, &mut output)?;

        if let Some(ev) = self.events.get(&original_total) {
            if !ev.inserts.is_empty() && output.wrote_content && !output.ended_with_terminator {
                let term = doc.default_terminator();
                w.write_all(term)?;
                output.mark_bytes(term);
            }
            for text in &ev.inserts {
                write_edited_line(&mut w, doc, text, doc.default_terminator(), &mut output)?;
            }
        }

        w.flush()?;
        w.get_ref().sync_all()?;
        drop(w);
        // Untouched runs were copied straight out of the mmap; if the base
        // file shrank mid-save those copies are zero-fill — abort before the
        // temp file replaces anything.
        if let Err(e) = doc.verify_base() {
            let _ = std::fs::remove_file(&tmp);
            return Err(e);
        }
        commit_temp_file(&tmp, target, overwrite)
    }
}

fn eol_text(eol: crate::Eol) -> &'static str {
    match eol {
        crate::Eol::Crlf => "\r\n",
        crate::Eol::Cr => "\r",
        crate::Eol::Lf | crate::Eol::Mixed | crate::Eol::None => "\n",
    }
}
struct ConvertedWriter<'a, W: Write> {
    enc: crate::Encoding,
    terminator: &'a [u8],
    total: u64,
    ends_nl: bool,
    logical: u64,
    w: &'a mut W,
}

impl<W: Write> ConvertedWriter<'_, W> {
    fn write_original_span(&mut self, doc: &Document, start: u64, end: u64) -> Result<()> {
        const BATCH: u64 = 8192;
        let mut pos = start;
        while pos < end {
            let batch = doc.raw_line_ranges(pos, (end - pos).min(BATCH));
            if batch.is_empty() {
                return Err(Error::InvalidInput(format!(
                    "original lines {}..{} are out of range while converting",
                    start + 1,
                    end
                )));
            }
            let advanced = batch.len() as u64;
            for (_line_no, raw) in batch {
                let text = doc.encoding().decode_line(raw);
                self.write_text(&text)?;
            }
            pos += advanced;
        }
        Ok(())
    }

    fn write_text(&mut self, text: &str) -> Result<()> {
        let line_no = self.logical + 1;
        let bytes = self.enc.encode_text(text).ok_or_else(|| {
            Error::InvalidInput(format!(
                "line {line_no} has characters that cannot be written as {}",
                self.enc.label()
            ))
        })?;
        self.w.write_all(&bytes)?;
        self.logical += 1;
        if self.logical < self.total || self.ends_nl {
            self.w.write_all(self.terminator)?;
        }
        Ok(())
    }
}

/// Copy original lines `[start, end)` (with their terminators) as one
/// contiguous byte range out of the mmap.
#[derive(Default)]
struct OutputState {
    wrote_content: bool,
    ended_with_terminator: bool,
}

impl OutputState {
    fn mark_bytes(&mut self, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }
        self.wrote_content = true;
        self.ended_with_terminator = bytes.ends_with(b"\n") || bytes.ends_with(b"\r");
    }

    fn mark_line(&mut self, terminator: &[u8]) {
        self.wrote_content = true;
        self.ended_with_terminator = !terminator.is_empty();
    }
}

fn copy_original_span(
    mut w: impl Write,
    doc: &Document,
    start: u64,
    end: u64,
    output: &mut OutputState,
) -> Result<()> {
    if start >= end {
        return Ok(());
    }
    let bytes = doc.raw_lines_span(start, end).ok_or_else(|| {
        Error::InvalidInput(format!(
            "original lines {}..{} are out of range while saving",
            start + 1,
            end
        ))
    })?;
    w.write_all(bytes)?;
    output.mark_bytes(bytes);
    Ok(())
}

fn write_edited_line(
    mut w: impl Write,
    doc: &Document,
    text: &str,
    terminator: &[u8],
    output: &mut OutputState,
) -> Result<()> {
    let bytes = doc.encoding().encode_text(text).ok_or_else(|| {
        Error::InvalidInput(format!(
            "edited text cannot be encoded as {}",
            doc.encoding().label()
        ))
    })?;
    w.write_all(&bytes)?;
    w.write_all(terminator)?;
    output.mark_line(terminator);
    Ok(())
}

/// Rename the fully-written temp file onto `target`. When `overwrite` is set
/// and the plain rename fails because `target` exists (Windows), preserve the
/// existing file under an aside name while promoting the temp file.
/// The temp file is cleaned up on any failure.
fn commit_temp_file(tmp: &Path, target: &Path, overwrite: bool) -> Result<()> {
    if overwrite {
        if let Err(e) = replace_with_staged(tmp, target) {
            let _ = std::fs::remove_file(tmp);
            return Err(Error::Io(e));
        }
        return Ok(());
    }

    match std::fs::rename(tmp, target) {
        Ok(()) => {
            fsync_parent(target);
            Ok(())
        }
        Err(e) => {
            let _ = std::fs::remove_file(tmp);
            Err(Error::Io(e))
        }
    }
}

/// True when the source file's last line carries a terminator, so a converting
/// save knows whether to write a trailing line ending after the final line.
fn document_ends_with_newline(doc: &Document) -> bool {
    let n = doc.line_count();
    n != 0 && doc.line_terminator(n - 1).is_some_and(|t| !t.is_empty())
}
