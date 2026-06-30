//! Sparse edit overlay for huge files.
//!
//! The base document remains an immutable mmap. Edits are stored as a small
//! line-oriented patch set keyed by original line number, then saved by streaming
//! original bytes plus patched fragments to a new file.

use std::collections::BTreeMap;
use std::fs::OpenOptions;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::{Document, Error, Result};

#[derive(Clone, Debug, Default)]
pub struct EditSession {
    events: BTreeMap<u64, EditEvent>,
    revision: u64,
}

#[derive(Clone, Debug, Default)]
struct EditEvent {
    /// Lines inserted before this original line. The special anchor
    /// `original_line_count` means "append after the original file".
    inserts: Vec<String>,
    replacement: Option<String>,
    deleted: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct EditLine {
    pub number: u64,
    pub text: String,
    pub edited: bool,
    pub inserted: bool,
    pub original_line: Option<u64>,
}

#[derive(Clone, Copy, Debug, Serialize)]
pub struct EditStats {
    pub dirty: bool,
    pub revision: u64,
    pub total_lines: u64,
    pub inserted_lines: u64,
    pub replaced_lines: u64,
    pub deleted_lines: u64,
}

#[derive(Clone, Debug, Serialize)]
pub struct SaveResult {
    pub path: PathBuf,
    pub bytes: u64,
    pub lines: u64,
}

enum LineRef {
    Original(u64),
    Replaced(u64),
    Inserted { anchor: u64, index: usize },
}

impl EditSession {
    pub fn is_dirty(&self) -> bool {
        !self.events.is_empty()
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn clear(&mut self) {
        if !self.events.is_empty() {
            self.events.clear();
            self.bump();
        }
    }

    pub fn stats(&self, doc: &Document) -> EditStats {
        let original = doc.line_count();
        let mut inserted = 0u64;
        let mut replaced = 0u64;
        let mut deleted = 0u64;
        for (&anchor, ev) in &self.events {
            inserted += ev.inserts.len() as u64;
            if anchor < original {
                if ev.deleted {
                    deleted += 1;
                } else if ev.replacement.is_some() {
                    replaced += 1;
                }
            }
        }
        EditStats {
            dirty: self.is_dirty(),
            revision: self.revision,
            total_lines: original + inserted - deleted,
            inserted_lines: inserted,
            replaced_lines: replaced,
            deleted_lines: deleted,
        }
    }

    pub fn total_lines(&self, doc: &Document) -> u64 {
        self.stats(doc).total_lines
    }

    pub fn lines(&self, doc: &Document, start: u64, count: u64) -> Vec<EditLine> {
        let total = self.total_lines(doc);
        let end = start.saturating_add(count).min(total);
        let mut out = Vec::with_capacity((end - start).min(4096) as usize);
        for logical in start..end {
            if let Some(line) = self.line(doc, logical) {
                out.push(line);
            }
        }
        out
    }

    pub fn line(&self, doc: &Document, logical: u64) -> Option<EditLine> {
        match self.locate(logical, doc.line_count())? {
            LineRef::Original(orig) => Some(EditLine {
                number: logical,
                text: doc.line(orig)?,
                edited: false,
                inserted: false,
                original_line: Some(orig),
            }),
            LineRef::Replaced(orig) => {
                let text = self.events.get(&orig)?.replacement.clone()?;
                Some(EditLine {
                    number: logical,
                    text,
                    edited: true,
                    inserted: false,
                    original_line: Some(orig),
                })
            }
            LineRef::Inserted { anchor, index } => {
                let text = self.events.get(&anchor)?.inserts.get(index)?.clone();
                Some(EditLine {
                    number: logical,
                    text,
                    edited: true,
                    inserted: true,
                    original_line: None,
                })
            }
        }
    }

    pub fn replace_line(&mut self, doc: &Document, logical: u64, text: String) -> Result<()> {
        match self
            .locate(logical, doc.line_count())
            .ok_or_else(|| Error::Unsupported(format!("line {} is out of range", logical + 1)))?
        {
            LineRef::Original(orig) | LineRef::Replaced(orig) => {
                let ev = self.events.entry(orig).or_default();
                ev.replacement = if doc.line(orig).as_deref() == Some(text.as_str()) {
                    None
                } else {
                    Some(text)
                };
                ev.deleted = false;
                self.clean_anchor(orig);
            }
            LineRef::Inserted { anchor, index } => {
                if let Some(line) = self
                    .events
                    .get_mut(&anchor)
                    .and_then(|ev| ev.inserts.get_mut(index))
                {
                    *line = text;
                }
                self.clean_anchor(anchor);
            }
        }
        self.bump();
        Ok(())
    }

    /// Insert `text` before logical line `logical`; `logical == total_lines`
    /// appends after the current document.
    pub fn insert_line_before(&mut self, doc: &Document, logical: u64, text: String) -> Result<()> {
        let total = self.total_lines(doc);
        if logical > total {
            return Err(Error::Unsupported(format!(
                "line {} is beyond end of document",
                logical + 1
            )));
        }
        if logical == total {
            self.events
                .entry(doc.line_count())
                .or_default()
                .inserts
                .push(text);
            self.bump();
            return Ok(());
        }

        match self.locate(logical, doc.line_count()).unwrap() {
            LineRef::Original(orig) | LineRef::Replaced(orig) => {
                self.events.entry(orig).or_default().inserts.push(text);
            }
            LineRef::Inserted { anchor, index } => {
                self.events
                    .entry(anchor)
                    .or_default()
                    .inserts
                    .insert(index, text);
            }
        }
        self.bump();
        Ok(())
    }

    pub fn delete_line(&mut self, doc: &Document, logical: u64) -> Result<()> {
        match self
            .locate(logical, doc.line_count())
            .ok_or_else(|| Error::Unsupported(format!("line {} is out of range", logical + 1)))?
        {
            LineRef::Original(orig) | LineRef::Replaced(orig) => {
                let ev = self.events.entry(orig).or_default();
                ev.replacement = None;
                ev.deleted = true;
                self.clean_anchor(orig);
            }
            LineRef::Inserted { anchor, index } => {
                if let Some(ev) = self.events.get_mut(&anchor) {
                    if index < ev.inserts.len() {
                        ev.inserts.remove(index);
                    }
                }
                self.clean_anchor(anchor);
            }
        }
        self.bump();
        Ok(())
    }

    pub fn save_to_path(&self, doc: &Document, target: impl AsRef<Path>) -> Result<SaveResult> {
        let target = target.as_ref();
        if target.exists() {
            return Err(Error::Unsupported(format!(
                "'{}' already exists; choose another save path",
                target.display()
            )));
        }
        self.write_stream(doc, target)?;
        let bytes = std::fs::metadata(target)?.len();
        Ok(SaveResult {
            path: target.to_path_buf(),
            bytes,
            lines: self.total_lines(doc),
        })
    }

    fn locate(&self, logical: u64, original_total: u64) -> Option<LineRef> {
        let mut logical_pos = 0u64;
        let mut orig = 0u64;

        for (&anchor, ev) in &self.events {
            let anchor = anchor.min(original_total);
            if anchor < orig {
                continue;
            }
            let unchanged = anchor - orig;
            if logical < logical_pos + unchanged {
                return Some(LineRef::Original(orig + (logical - logical_pos)));
            }
            logical_pos += unchanged;
            orig = anchor;

            let inserted = ev.inserts.len() as u64;
            if logical < logical_pos + inserted {
                return Some(LineRef::Inserted {
                    anchor,
                    index: (logical - logical_pos) as usize,
                });
            }
            logical_pos += inserted;

            if anchor < original_total {
                if ev.deleted {
                    orig += 1;
                } else {
                    if logical == logical_pos {
                        return if ev.replacement.is_some() {
                            Some(LineRef::Replaced(anchor))
                        } else {
                            Some(LineRef::Original(anchor))
                        };
                    }
                    logical_pos += 1;
                    orig += 1;
                }
            }
        }

        let unchanged = original_total - orig;
        if logical < logical_pos + unchanged {
            Some(LineRef::Original(orig + (logical - logical_pos)))
        } else {
            None
        }
    }

    fn write_stream(&self, doc: &Document, target: &Path) -> Result<()> {
        let tmp = temp_path(target);
        let file = OpenOptions::new().write(true).create_new(true).open(&tmp)?;
        let mut w = BufWriter::new(file);

        w.write_all(doc.prefix_bytes())?;
        let original_total = doc.line_count();
        for orig in 0..original_total {
            if let Some(ev) = self.events.get(&orig) {
                for text in &ev.inserts {
                    write_edited_line(&mut w, doc, text, doc.default_terminator())?;
                }
                if ev.deleted {
                    continue;
                }
                if let Some(text) = &ev.replacement {
                    let term = doc.line_terminator(orig).unwrap_or(b"");
                    write_edited_line(&mut w, doc, text, term)?;
                } else if let Some(bytes) = doc.raw_line_with_terminator(orig) {
                    w.write_all(bytes)?;
                }
            } else if let Some(bytes) = doc.raw_line_with_terminator(orig) {
                w.write_all(bytes)?;
            }
        }

        if let Some(ev) = self.events.get(&original_total) {
            if !ev.inserts.is_empty()
                && original_total > 0
                && doc
                    .line_terminator(original_total - 1)
                    .unwrap_or(b"")
                    .is_empty()
            {
                w.write_all(doc.default_terminator())?;
            }
            for text in &ev.inserts {
                write_edited_line(&mut w, doc, text, doc.default_terminator())?;
            }
        }

        w.flush()?;
        w.get_ref().sync_all()?;
        drop(w);
        match std::fs::rename(&tmp, target) {
            Ok(()) => Ok(()),
            Err(e) => {
                let _ = std::fs::remove_file(&tmp);
                Err(Error::Io(e))
            }
        }
    }

    fn clean_anchor(&mut self, anchor: u64) {
        let should_remove = self
            .events
            .get(&anchor)
            .map(|ev| ev.inserts.is_empty() && ev.replacement.is_none() && !ev.deleted)
            .unwrap_or(false);
        if should_remove {
            self.events.remove(&anchor);
        }
    }

    fn bump(&mut self) {
        self.revision = self.revision.wrapping_add(1);
    }
}

fn write_edited_line(
    mut w: impl Write,
    doc: &Document,
    text: &str,
    terminator: &[u8],
) -> Result<()> {
    let bytes = doc.encoding().encode_text(text).ok_or_else(|| {
        Error::Unsupported(format!(
            "edited text cannot be encoded as {}",
            doc.encoding().label()
        ))
    })?;
    w.write_all(&bytes)?;
    w.write_all(terminator)?;
    Ok(())
}

fn temp_path(target: &Path) -> PathBuf {
    let parent = target.parent().unwrap_or_else(|| Path::new("."));
    let name = target
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("ayame-save");
    parent.join(format!(
        ".{name}.ayame-tmp-{}-{}",
        std::process::id(),
        unique_suffix()
    ))
}

fn unique_suffix() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use tempfile::NamedTempFile;

    use super::*;
    use crate::Encoding;
    use crate::OpenOptions as AyameOpenOptions;

    fn doc_from(bytes: &[u8]) -> (NamedTempFile, Document) {
        let mut f = NamedTempFile::new().unwrap();
        f.write_all(bytes).unwrap();
        let doc = Document::open(f.path(), &AyameOpenOptions::default()).unwrap();
        (f, doc)
    }

    fn doc_from_with_options(bytes: &[u8], opts: AyameOpenOptions) -> (NamedTempFile, Document) {
        let mut f = NamedTempFile::new().unwrap();
        f.write_all(bytes).unwrap();
        let doc = Document::open(f.path(), &opts).unwrap();
        (f, doc)
    }

    #[test]
    fn line_overlay_replaces_inserts_and_deletes() {
        let (_f, doc) = doc_from(b"a\nb\nc\n");
        let mut edits = EditSession::default();
        edits.replace_line(&doc, 1, "B".into()).unwrap();
        edits.insert_line_before(&doc, 2, "x".into()).unwrap();
        edits.delete_line(&doc, 0).unwrap();
        let lines: Vec<_> = edits
            .lines(&doc, 0, 10)
            .into_iter()
            .map(|l| l.text)
            .collect();
        assert_eq!(lines, vec!["B", "x", "c"]);
        let st = edits.stats(&doc);
        assert_eq!(st.total_lines, 3);
        assert_eq!(st.replaced_lines, 1);
        assert_eq!(st.inserted_lines, 1);
        assert_eq!(st.deleted_lines, 1);
    }

    #[test]
    fn save_stream_preserves_untouched_bytes_and_crlf() {
        let (f, doc) = doc_from(b"a\r\nb\r\nc");
        let out = f.path().with_extension("out");
        let mut edits = EditSession::default();
        edits.replace_line(&doc, 1, "B".into()).unwrap();
        edits.insert_line_before(&doc, 3, "d".into()).unwrap();
        edits.save_to_path(&doc, &out).unwrap();
        assert_eq!(std::fs::read(&out).unwrap(), b"a\r\nB\r\nc\r\nd\r\n");
        let _ = std::fs::remove_file(out);
    }

    #[test]
    fn replacing_with_original_text_clears_dirty_state() {
        let (f, doc) = doc_from(b"a\nb\n");
        let out = f.path().with_extension("same");
        let mut edits = EditSession::default();
        edits.replace_line(&doc, 1, "B".into()).unwrap();
        assert!(edits.is_dirty());
        edits.replace_line(&doc, 1, "b".into()).unwrap();
        assert!(!edits.is_dirty());
        edits.save_to_path(&doc, &out).unwrap();
        assert_eq!(std::fs::read(&out).unwrap(), b"a\nb\n");
        let _ = std::fs::remove_file(out);
    }

    #[test]
    fn empty_file_can_be_edited_and_saved() {
        let (f, doc) = doc_from(b"");
        let out = f.path().with_extension("inserted");
        let mut edits = EditSession::default();
        edits.insert_line_before(&doc, 0, "alpha".into()).unwrap();
        edits.insert_line_before(&doc, 1, "beta".into()).unwrap();
        edits.save_to_path(&doc, &out).unwrap();
        assert_eq!(edits.stats(&doc).total_lines, 2);
        assert_eq!(std::fs::read(&out).unwrap(), b"alpha\nbeta\n");
        let _ = std::fs::remove_file(out);
    }

    #[test]
    fn shift_jis_edits_are_encoded_while_untouched_bytes_are_preserved() {
        let opts = AyameOpenOptions {
            encoding: Some(Encoding::ShiftJis),
            ..AyameOpenOptions::default()
        };
        let (f, doc) = doc_from_with_options(b"\x82\xa0\r\nraw\xff\r\n", opts);
        let out = f.path().with_extension("sjis");
        let mut edits = EditSession::default();
        edits.replace_line(&doc, 0, "い".into()).unwrap();
        edits.save_to_path(&doc, &out).unwrap();
        assert_eq!(std::fs::read(&out).unwrap(), b"\x82\xa2\r\nraw\xff\r\n");
        let _ = std::fs::remove_file(out);
    }
}
