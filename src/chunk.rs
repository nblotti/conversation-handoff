const TARGET_CHARS: usize = 700;

/// Split stored context into retrieval-sized pieces.
/// Prefers paragraph breaks, then whitespace, so later ranking can pick
/// only the parts that match the latest message or a recall query.
pub fn chunk_text(text: &str) -> Vec<String> {
    let mut chunks = Vec::new();
    for para in text.split("\n\n") {
        let para = para.trim();
        if para.is_empty() {
            continue;
        }
        if para.chars().count() <= TARGET_CHARS + TARGET_CHARS / 2 {
            chunks.push(para.to_string());
        } else {
            chunks.extend(split_long(para, TARGET_CHARS));
        }
    }
    if chunks.is_empty() {
        let trimmed = text.trim();
        if !trimmed.is_empty() {
            chunks.push(trimmed.to_string());
        }
    }
    chunks
}

fn split_long(text: &str, max_chars: usize) -> Vec<String> {
    let mut out = Vec::new();
    let chars: Vec<char> = text.chars().collect();
    let mut start = 0;
    while start < chars.len() {
        let mut end = (start + max_chars).min(chars.len());
        if end < chars.len() {
            if let Some(rel) = chars[start..end].iter().rposition(|c| c.is_whitespace()) {
                end = start + rel;
            }
        }
        if end == start {
            end = (start + max_chars).min(chars.len());
        }
        let piece: String = chars[start..end].iter().collect();
        let piece = piece.trim();
        if !piece.is_empty() {
            out.push(piece.to_string());
        }
        start = end;
        while start < chars.len() && chars[start].is_whitespace() {
            start += 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_paragraphs() {
        let chunks = chunk_text("alpha\n\nbeta\n\n\ngamma");
        assert_eq!(chunks, vec!["alpha", "beta", "gamma"]);
    }

    #[test]
    fn splits_long_paragraphs() {
        let word = "word ";
        let long = word.repeat(400);
        let chunks = chunk_text(&long);
        assert!(chunks.len() > 1);
        assert!(chunks.iter().all(|c| c.chars().count() <= TARGET_CHARS + 5));
    }
}
