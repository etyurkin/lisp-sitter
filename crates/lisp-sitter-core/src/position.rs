pub fn line_column(content: &str, pos: usize) -> (usize, usize) {
    let pos = pos.min(content.len());
    let mut line = 1usize;
    let mut col = 1usize;
    for (i, ch) in content.char_indices() {
        if i >= pos {
            break;
        }
        if ch == '\n' {
            line += 1;
            col = 1;
        } else if ch != '\r' {
            col += 1;
        }
    }
    (line, col)
}

/// Precomputed line-start byte offsets for one buffer, so repeated position
/// lookups are `O(log lines + line length)` instead of `O(pos)` each. Build it
/// once when labeling many positions in the same file (outlines, diffs,
/// analysis findings) rather than calling [`line_column`] per position.
pub struct LineIndex<'a> {
    content: &'a str,
    /// Byte offset of the start of each line (line 1 starts at `line_starts[0]`).
    line_starts: Vec<usize>,
}

impl<'a> LineIndex<'a> {
    pub fn new(content: &'a str) -> Self {
        let mut line_starts = vec![0usize];
        for (i, b) in content.bytes().enumerate() {
            if b == b'\n' {
                line_starts.push(i + 1);
            }
        }
        Self {
            content,
            line_starts,
        }
    }

    /// 1-based `(line, column)` of `pos`. Column counts characters from the line
    /// start (skipping `\r`), matching [`line_column`].
    pub fn locate(&self, pos: usize) -> (usize, usize) {
        let pos = pos.min(self.content.len());
        // Largest line whose start is <= pos.
        let line = self.line_starts.partition_point(|&s| s <= pos).max(1);
        let line_start = self.line_starts[line - 1];
        let col = 1 + self.content[line_start..pos]
            .chars()
            .filter(|&c| c != '\r')
            .count();
        (line, col)
    }

    pub fn label(&self, pos: usize, label: &str) -> String {
        let (line, col) = self.locate(pos);
        format!("{label}@{line}:{col}")
    }
}

#[cfg(test)]
mod tests {
    use super::{line_column, LineIndex};

    #[test]
    fn line_index_matches_line_column() {
        for content in [
            "",
            "abc",
            "a\nbc\n\ndef",
            "(defun f ()\r\n  1)\r\n",
            "α\nβγ\nδ",
        ] {
            let index = LineIndex::new(content);
            for pos in 0..=content.len() {
                if !content.is_char_boundary(pos) {
                    continue;
                }
                assert_eq!(
                    index.locate(pos),
                    line_column(content, pos),
                    "mismatch at pos {pos} in {content:?}"
                );
            }
        }
    }
}

pub fn error_at(content: &str, pos: usize, message: impl AsRef<str>) -> String {
    let (line, col) = line_column(content, pos);
    format!("line {line}, column {col}: {}", message.as_ref())
}

pub fn pos_label(content: &str, pos: usize, label: &str) -> String {
    let (line, col) = line_column(content, pos);
    format!("{label}@{line}:{col}")
}
