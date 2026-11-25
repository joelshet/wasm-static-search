const K1: f32 = 1.2;
/// How much a long document is marked down. Classic BM25 uses 0.75; a site mixes one-line pages with long
/// ones, and at 0.75 a long page that says a word ten times loses to a stub that says it once.
const B: f32 = 0.4;

/// How rare a term is: higher the fewer of the `n` documents hold it.
pub fn idf(n: usize, df: usize) -> f32 {
    let (n, df) = (n as f32, df as f32);
    ((n - df + 0.5) / (df + 0.5) + 1.0).ln()
}

/// How much one document says a term. Repeats count for less each time, and length is evened out.
pub fn saturation(tf: f32, len: f32, avg_len: f32) -> f32 {
    tf * (K1 + 1.0) / (tf + K1 * (1.0 - B + B * len / avg_len))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rare_term_scores_higher() {
        assert!(idf(10, 1) > idf(10, 10));
    }

    #[test]
    fn repeats_count_for_less_each_time() {
        let once = saturation(1.0, 20.0, 20.0);
        let twice = saturation(2.0, 20.0, 20.0);
        let thrice = saturation(3.0, 20.0, 20.0);
        assert!(twice > once && thrice - twice < twice - once);
    }

    #[test]
    fn a_longer_document_scores_lower_for_the_same_count() {
        assert!(saturation(2.0, 200.0, 20.0) < saturation(2.0, 20.0, 20.0));
    }
}
