use std::collections::HashMap;

const K1: f32 = 1.5;
const B: f32 = 0.75;

/// Tiny BM25-style ranker. No embeddings, no network: good enough to pick
/// the chunks that overlap the latest message or a follow-up question.
/// Returns `(score, chunk_index)` sorted by score descending.
pub fn rank(chunks: &[String], query: &str) -> Vec<(f32, usize)> {
    let query_terms = tokenize(query);
    if chunks.is_empty() || query_terms.is_empty() {
        return Vec::new();
    }

    let docs: Vec<Vec<String>> = chunks.iter().map(|c| tokenize(c)).collect();
    let n = docs.len() as f32;
    let avgdl = docs.iter().map(|d| d.len() as f32).sum::<f32>() / n;

    let mut df: HashMap<&str, u32> = HashMap::new();
    for doc in &docs {
        let mut seen = HashMap::new();
        for term in doc {
            seen.insert(term.as_str(), ());
        }
        for term in seen.keys() {
            *df.entry(*term).or_insert(0) += 1;
        }
    }

    let mut scored: Vec<(f32, usize)> = docs
        .iter()
        .enumerate()
        .map(|(idx, doc)| {
            let mut tf: HashMap<&str, u32> = HashMap::new();
            for term in doc {
                *tf.entry(term.as_str()).or_insert(0) += 1;
            }
            let dl = doc.len() as f32;
            let mut score = 0.0;
            for term in &query_terms {
                let f = *tf.get(term.as_str()).unwrap_or(&0) as f32;
                if f == 0.0 {
                    continue;
                }
                let n_qi = *df.get(term.as_str()).unwrap_or(&0) as f32;
                let idf = ((n - n_qi + 0.5) / (n_qi + 0.5) + 1.0).ln();
                let denom = f + K1 * (1.0 - B + B * dl / avgdl.max(1.0));
                score += idf * (f * (K1 + 1.0)) / denom;
            }
            (score, idx)
        })
        .filter(|(score, _)| *score > 0.0)
        .collect();

    scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    scored
}

pub fn tokenize(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .map(|t| t.to_lowercase())
        .filter(|t| t.len() > 1 && !is_stop(t))
        .collect()
}

fn is_stop(term: &str) -> bool {
    matches!(
        term,
        "a" | "an"
            | "and"
            | "are"
            | "as"
            | "at"
            | "be"
            | "but"
            | "by"
            | "for"
            | "from"
            | "has"
            | "have"
            | "if"
            | "in"
            | "into"
            | "is"
            | "it"
            | "its"
            | "of"
            | "on"
            | "or"
            | "that"
            | "the"
            | "this"
            | "to"
            | "was"
            | "were"
            | "will"
            | "with"
            | "you"
            | "your"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefers_latest_topic() {
        let chunks = vec![
            "We refactored the billing module last week and renamed invoices.".to_string(),
            "The auth tests fail because the JWT secret is missing in CI.".to_string(),
            "Updated README screenshots.".to_string(),
        ];
        let ranked = rank(&chunks, "jwt secret missing in the auth tests");
        assert!(!ranked.is_empty());
        assert!(chunks[ranked[0].1].contains("JWT secret"));
    }

    #[test]
    fn empty_query_returns_nothing() {
        let chunks = vec!["hello world".to_string()];
        assert!(rank(&chunks, "   ").is_empty());
    }
}
