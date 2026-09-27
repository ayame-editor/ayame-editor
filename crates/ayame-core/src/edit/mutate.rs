//! Overlay mutators: single-line, range, batch, and rectangle edits.

use super::*;

impl EditSession {
    /// Commit `record` as one undo generation and, only when it actually
    /// changed the overlay, mirror the op into the attached crash log. The
    /// closure is monomorphized and invoked only with a writer attached, so
    /// the no-WAL hot path keeps its allocation-free shape.
    fn commit_record(&mut self, record: UndoRecord, logged: impl FnOnce() -> LoggedOp) {
        if self.finish_change(record) {
            self.wal_commit(logged);
        }
    }

    pub fn replace_line(&mut self, doc: &Document, logical: u64, text: String) -> Result<()> {
        // Pre-clone the text for the log only when a WAL is attached (the
        // inner call consumes it); the no-WAL path stays allocation-free.
        let logged = self.wal.is_some().then(|| text.clone());
        let mut record = UndoRecord::new();
        self.replace_line_inner(doc, logical, text, &mut record)?;
        self.commit_record(record, || LoggedOp::ReplaceLine {
            line: logical,
            text: logged.unwrap_or_default(),
        });
        Ok(())
    }

    /// Mutation without committing undo history (composed by `replace_range`).
    /// Appends the inverse of every actual state change to `record`.
    fn replace_line_inner(
        &mut self,
        doc: &Document,
        logical: u64,
        text: String,
        record: &mut UndoRecord,
    ) -> Result<()> {
        match self
            .locate(logical, doc.line_count())
            .ok_or_else(|| Error::InvalidInput(format!("line {} is out of range", logical + 1)))?
        {
            LineRef::Original(orig) | LineRef::Replaced(orig) => {
                // The no-op shortcut compares against the *view* text, so it
                // must not fire for lines past the view cap: the capped text
                // could coincidentally equal the replacement while the real
                // line differs beyond the cap (#201).
                let over_cap = doc
                    .line_byte_len(orig)
                    .is_some_and(|len| len > Document::MAX_VIEW_LINE_BYTES as u64);
                let replacement = if !over_cap && doc.line(orig).as_deref() == Some(text.as_str()) {
                    None
                } else {
                    Some(text)
                };
                let prior = self.events.get(&orig);
                let prior_replacement = prior.and_then(|ev| ev.replacement.clone());
                let prior_deleted = prior.is_some_and(|ev| ev.deleted);
                if prior_replacement == replacement && !prior_deleted {
                    // No state change; the end state is identical either way.
                    return Ok(());
                }
                record.push(UndoOp::SetLineState {
                    anchor: orig,
                    replacement: prior_replacement,
                    deleted: prior_deleted,
                });
                let ev = self.events.entry(orig).or_default();
                ev.replacement = replacement;
                ev.deleted = false;
                self.clean_anchor(orig);
            }
            LineRef::Inserted { anchor, index } => {
                if let Some(line) = self
                    .events
                    .get_mut(&anchor)
                    .and_then(|ev| ev.inserts.get_mut(index))
                {
                    if *line != text {
                        record.push(UndoOp::SetInsert {
                            anchor,
                            index,
                            text: std::mem::replace(line, text),
                        });
                    }
                }
                self.clean_anchor(anchor);
            }
        }
        Ok(())
    }

    /// Insert `text` before logical line `logical`; `logical == total_lines`
    /// appends after the current document.
    pub fn insert_line_before(&mut self, doc: &Document, logical: u64, text: String) -> Result<()> {
        let logged = self.wal.is_some().then(|| text.clone());
        let mut record = UndoRecord::new();
        self.insert_line_before_inner(doc, logical, text, &mut record)?;
        self.commit_record(record, || LoggedOp::InsertLine {
            line: logical,
            text: logged.unwrap_or_default(),
        });
        Ok(())
    }

    fn insert_line_before_inner(
        &mut self,
        doc: &Document,
        logical: u64,
        text: String,
        record: &mut UndoRecord,
    ) -> Result<()> {
        let total = self.total_lines(doc);
        if logical > total {
            return Err(Error::InvalidInput(format!(
                "line {} is beyond end of document",
                logical + 1
            )));
        }
        if logical == total {
            let anchor = doc.line_count();
            let ev = self.events.entry(anchor).or_default();
            record.push(UndoOp::RemoveInsert {
                anchor,
                index: ev.inserts.len(),
            });
            ev.inserts.push(text);
            return Ok(());
        }
        match self.locate(logical, doc.line_count()).unwrap() {
            LineRef::Original(orig) | LineRef::Replaced(orig) => {
                let ev = self.events.entry(orig).or_default();
                record.push(UndoOp::RemoveInsert {
                    anchor: orig,
                    index: ev.inserts.len(),
                });
                ev.inserts.push(text);
            }
            LineRef::Inserted { anchor, index } => {
                record.push(UndoOp::RemoveInsert { anchor, index });
                self.events
                    .entry(anchor)
                    .or_default()
                    .inserts
                    .insert(index, text);
            }
        }
        Ok(())
    }

    pub fn delete_line(&mut self, doc: &Document, logical: u64) -> Result<()> {
        let mut record = UndoRecord::new();
        self.delete_line_inner(doc, logical, &mut record)?;
        self.commit_record(record, || LoggedOp::DeleteLine { line: logical });
        Ok(())
    }

    fn delete_line_inner(
        &mut self,
        doc: &Document,
        logical: u64,
        record: &mut UndoRecord,
    ) -> Result<()> {
        match self
            .locate(logical, doc.line_count())
            .ok_or_else(|| Error::InvalidInput(format!("line {} is out of range", logical + 1)))?
        {
            LineRef::Original(orig) | LineRef::Replaced(orig) => {
                let prior = self.events.get(&orig);
                let prior_replacement = prior.and_then(|ev| ev.replacement.clone());
                let prior_deleted = prior.is_some_and(|ev| ev.deleted);
                if prior_replacement.is_some() || !prior_deleted {
                    record.push(UndoOp::SetLineState {
                        anchor: orig,
                        replacement: prior_replacement,
                        deleted: prior_deleted,
                    });
                }
                let ev = self.events.entry(orig).or_default();
                ev.replacement = None;
                ev.deleted = true;
                self.clean_anchor(orig);
            }
            LineRef::Inserted { anchor, index } => {
                if let Some(ev) = self.events.get_mut(&anchor) {
                    if index < ev.inserts.len() {
                        let removed = ev.inserts.remove(index);
                        record.push(UndoOp::InsertInsert {
                            anchor,
                            index,
                            text: removed,
                        });
                    }
                }
                self.clean_anchor(anchor);
            }
        }
        Ok(())
    }

    /// Replace the logical span (l0,c0)..(l1,c1) with `text` (which may contain
    /// '\n'), as a SINGLE undo unit. Column offsets are Unicode scalar (char)
    /// counts into the decoded line text. Returns the caret (line, col) after
    /// the edit. `replace_range(l,c,l,c,text)` is a plain insert at (l,c).
    pub fn replace_range(
        &mut self,
        doc: &Document,
        l0: u64,
        c0: usize,
        l1: u64,
        c1: usize,
        text: &str,
    ) -> Result<(u64, usize)> {
        let mut record = UndoRecord::new();
        let caret = self.replace_range_inner(doc, l0, c0, l1, c1, text, &mut record)?;
        self.commit_record(record, || LoggedOp::ReplaceRange {
            l0,
            c0,
            l1,
            c1,
            text: text.to_string(),
        });
        Ok(caret)
    }

    /// The body of [`EditSession::replace_range`] without the history commit:
    /// every inverse is appended to `record`, so several range replacements
    /// (one per caret) can be composed into a single undo step.
    // Mirrors the public 7-argument signature plus the composed record.
    #[allow(clippy::too_many_arguments)]
    fn replace_range_inner(
        &mut self,
        doc: &Document,
        l0: u64,
        c0: usize,
        l1: u64,
        c1: usize,
        text: &str,
        record: &mut UndoRecord,
    ) -> Result<(u64, usize)> {
        let total = self.total_lines(doc);
        // An empty document has no lines at all; the only valid edit is an
        // insertion at the very start, which seeds the first line(s).
        if total == 0 {
            if l0 != 0 || c0 != 0 || l1 != 0 || c1 != 0 {
                return Err(Error::InvalidInput(
                    "the document is empty; only an insertion at line 1 is valid".into(),
                ));
            }
            let parts: Vec<String> = text.split('\n').map(String::from).collect();
            for (k, p) in parts.iter().enumerate() {
                self.insert_line_before_inner(doc, k as u64, p.clone(), record)?;
            }
            let last = parts.len() - 1;
            return Ok((last as u64, parts[last].chars().count()));
        }
        if l0 > l1 || l1 >= total {
            return Err(Error::InvalidInput(format!(
                "range spans lines {}..{} outside the document",
                l0 + 1,
                l1 + 1
            )));
        }
        let first = self.line_text(doc, l0)?;
        let last = if l1 == l0 {
            first.clone()
        } else {
            self.line_text(doc, l1)?
        };
        let c0 = c0.min(first.chars().count());
        let c1 = c1.min(last.chars().count());
        let head: String = first.chars().take(c0).collect();
        let tail: String = last.chars().skip(c1).collect();

        let mut parts: Vec<String> = text.split('\n').map(String::from).collect();
        let n = parts.len();
        let li = n - 1;
        parts[0] = format!("{head}{}", parts[0]);
        parts[li] = format!("{}{tail}", parts[li]);

        // Delete the interior lines descending so anchors above the cursor
        // don't shift under us.
        self.replace_line_inner(doc, l0, parts[0].clone(), record)?;
        for l in ((l0 + 1)..=l1).rev() {
            self.delete_line_inner(doc, l, record)?;
        }
        for (k, p) in parts[1..].iter().enumerate() {
            self.insert_line_before_inner(doc, l0 + 1 + k as u64, p.clone(), record)?;
        }

        let caret_line = l0 + (n as u64 - 1);
        let caret_col = parts[li].chars().count() - tail.chars().count();
        Ok((caret_line, caret_col))
    }

    /// Apply one edit per caret — a multi-cursor commit — as a SINGLE undo
    /// step. Each entry replaces its span exactly like
    /// [`EditSession::replace_range`]; all coordinates refer to the shared
    /// view BEFORE the batch (the caller's simultaneous carets), and the
    /// ranges must not overlap. Returns the post-batch caret for every edit
    /// in request order: the position just past that edit's inserted text
    /// once every edit has been applied. The revision bumps once for the
    /// whole batch; a batch with no visible effect records nothing, exactly
    /// like the single-edit no-op detection.
    pub fn replace_batch(
        &mut self,
        doc: &Document,
        edits: &[BatchEdit],
    ) -> Result<Vec<(u64, usize)>> {
        if edits.is_empty() {
            return Ok(Vec::new());
        }
        let total = self.total_lines(doc);
        // Validate and clamp every range against the shared pre-batch view
        // up front, so the overlap check and the caret math below agree on
        // one coordinate space and nothing mutates until all edits are known
        // to be applicable.
        let mut clamped: Vec<(u64, usize, u64, usize)> = Vec::with_capacity(edits.len());
        for e in edits {
            if total == 0 {
                if e.l0 != 0 || e.c0 != 0 || e.l1 != 0 || e.c1 != 0 {
                    return Err(Error::InvalidInput(
                        "the document is empty; only an insertion at line 1 is valid".into(),
                    ));
                }
                clamped.push((0, 0, 0, 0));
                continue;
            }
            if e.l0 > e.l1 || e.l1 >= total {
                return Err(Error::InvalidInput(format!(
                    "range spans lines {}..{} outside the document",
                    e.l0 + 1,
                    e.l1 + 1
                )));
            }
            let first_len = self.line_text(doc, e.l0)?.chars().count();
            let last_len = if e.l1 == e.l0 {
                first_len
            } else {
                self.line_text(doc, e.l1)?.chars().count()
            };
            let c0 = e.c0.min(first_len);
            let c1 = e.c1.min(last_len);
            if e.l0 == e.l1 && c0 > c1 {
                return Err(Error::InvalidInput(format!(
                    "range on line {} is reversed (column {} comes after {})",
                    e.l0 + 1,
                    e.c0 + 1,
                    e.c1 + 1
                )));
            }
            clamped.push((e.l0, c0, e.l1, c1));
        }
        let mut order: Vec<usize> = (0..edits.len()).collect();
        order.sort_by_key(|&i| clamped[i]);
        for w in order.windows(2) {
            let (.., al1, ac1) = clamped[w[0]];
            let (bl0, bc0, ..) = clamped[w[1]];
            if (bl0, bc0) < (al1, ac1) {
                return Err(Error::InvalidInput(format!(
                    "batch edits overlap around line {}",
                    bl0 + 1
                )));
            }
        }

        // Apply bottom-most first: an application only touches text at or
        // after its own start, so the still-pending (smaller) coordinates
        // keep meaning what the caller meant. Every inverse lands in ONE
        // record, making the whole batch a single undo generation.
        let mut record = UndoRecord::new();
        for &i in order.iter().rev() {
            let (l0, c0, l1, c1) = clamped[i];
            if let Err(e) =
                self.replace_range_inner(doc, l0, c0, l1, c1, &edits[i].text, &mut record)
            {
                // Unreachable after the validation above, but never leave a
                // half-applied batch behind: unwind the partial record (the
                // overlay returns to its pre-batch state, no history entry).
                let _ = self.apply_record(record);
                return Err(e);
            }
        }
        self.commit_record(record, || LoggedOp::Batch {
            edits: edits.to_vec(),
        });

        // Map each caret into post-batch coordinates by replaying ascending:
        // `line_delta` accumulates the line-count change of everything above,
        // and the previous edit's end point carries the column shift for a
        // following edit that starts on the same original line.
        let mut carets = vec![(0u64, 0usize); edits.len()];
        let mut line_delta: i64 = 0;
        let mut prev_end: Option<((u64, usize), (u64, usize))> = None; // (old pos, new pos)
        for &i in &order {
            let (l0, c0, l1, c1) = clamped[i];
            let (start_line, start_col) = match prev_end {
                Some(((ol, oc), (nl, nc))) if ol == l0 => (nl, nc + (c0 - oc)),
                _ => ((l0 as i64 + line_delta) as u64, c0),
            };
            let parts: Vec<&str> = edits[i].text.split('\n').collect();
            let end_line = start_line + (parts.len() as u64 - 1);
            let end_col = if parts.len() == 1 {
                start_col + parts[0].chars().count()
            } else {
                parts[parts.len() - 1].chars().count()
            };
            carets[i] = (end_line, end_col);
            line_delta += parts.len() as i64 - 1 - (l1 as i64 - l0 as i64);
            prev_end = Some(((l1, c1), (end_line, end_col)));
        }
        Ok(carets)
    }

    /// Replace the same column span on every line in `[l0, l1]` as one undo
    /// unit. Multi-line `text` is mapped row-by-row, which is what rectangular
    /// paste expects.
    pub fn replace_rect(
        &mut self,
        doc: &Document,
        l0: u64,
        l1: u64,
        c0: usize,
        c1: usize,
        text: &str,
    ) -> Result<(u64, usize)> {
        let total = self.total_lines(doc);
        if total == 0 {
            return Err(Error::InvalidInput(
                "rectangular selection is not valid in an empty document".into(),
            ));
        }
        let top = l0.min(l1);
        let bottom = l0.max(l1);
        if bottom >= total {
            return Err(Error::InvalidInput(format!(
                "rectangle spans lines {}..{} outside the document",
                top + 1,
                bottom + 1
            )));
        }
        let left = c0.min(c1);
        let right = c0.max(c1);
        let parts: Vec<&str> = text.split('\n').collect();
        let mut record = UndoRecord::new();
        for line in top..=bottom {
            let replacement = if parts.len() == 1 {
                parts[0]
            } else {
                parts.get((line - top) as usize).copied().unwrap_or("")
            };
            let original = self.line_text(doc, line)?;
            let len = original.chars().count();
            let start = left.min(len);
            let end = right.min(len);
            let head: String = original.chars().take(start).collect();
            let tail: String = original.chars().skip(end).collect();
            self.replace_line_inner(doc, line, format!("{head}{replacement}{tail}"), &mut record)?;
        }
        self.commit_record(record, || LoggedOp::ReplaceRect {
            l0,
            l1,
            c0,
            c1,
            text: text.to_string(),
        });
        let caret_line = if parts.len() == 1 {
            bottom
        } else {
            let last_row = parts.len().saturating_sub(1) as u64;
            top + last_row.min(bottom - top)
        };
        let caret_text = parts
            .get((caret_line - top) as usize)
            .or_else(|| parts.first())
            .copied()
            .unwrap_or("");
        Ok((caret_line, left + caret_text.chars().count()))
    }
}
