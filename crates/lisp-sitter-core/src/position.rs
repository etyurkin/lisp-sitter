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

pub fn error_at(content: &str, pos: usize, message: impl AsRef<str>) -> String {
    let (line, col) = line_column(content, pos);
    format!("line {line}, column {col}: {}", message.as_ref())
}

pub fn pos_label(content: &str, pos: usize, label: &str) -> String {
    let (line, col) = line_column(content, pos);
    format!("{label}@{line}:{col}")
}
