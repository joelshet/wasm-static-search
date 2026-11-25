/// One word of a text: the form it is matched by, the form a corpus's tables read, and the bytes it covers.
pub struct Word {
    /// Lower case, without accents, and without the apostrophe inside it ("don't" is "dont").
    pub term: String,
    /// The same, keeping an apostrophe inside the word as `'`, so tables can tell "lov'd" from "lovd".
    pub form: String,
    pub start: usize,
    pub end: usize,
}

/// Split a text into words. Every word is kept, common ones included, so any word of a text can be searched for.
pub fn words(text: &str) -> Vec<Word> {
    let mut out = Vec::new();
    each_word(text, |term, form, start, end| out.push(Word { term: term.to_string(), form: form.to_string(), start, end }));
    out
}

/// Call `f` with each word's term, form, first byte and the byte after its last, reusing one buffer for all.
pub fn each_word(text: &str, mut f: impl FnMut(&str, &str, usize, usize)) {
    let (mut term, mut form) = (String::new(), String::new());
    let (mut start, mut end) = (0, 0);
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        // ASCII is most text; anything else is decoded as a char.
        let (c, width) = if b < 0x80 { (b as char, 1) } else { text[i..].chars().next().map(|c| (c, c.len_utf8())).unwrap() };
        if is_letter(c) {
            if term.is_empty() {
                start = i;
            }
            if b < 0x80 {
                let lower = b.to_ascii_lowercase() as char;
                term.push(lower);
                form.push(lower);
            } else {
                let at = term.len();
                fold(c, &mut term);
                form.push_str(&term[at..]);
            }
            end = i + width;
        } else if is_apostrophe(c) && !term.is_empty() && text[i + width..].chars().next().is_some_and(is_letter) {
            form.push('\'');
        } else if !term.is_empty() {
            f(&term, &form, start, end);
            term.clear();
            form.clear();
        }
        i += width;
    }
    if !term.is_empty() {
        f(&term, &form, start, end);
    }
}

/// A text's words as matched, joined by single spaces: how names and field values are compared.
pub fn plain(text: &str) -> String {
    words(text).into_iter().map(|w| w.term).collect::<Vec<_>>().join(" ")
}

fn is_apostrophe(c: char) -> bool {
    matches!(c, '\'' | '\u{2019}' | '\u{2018}' | '\u{02BB}')
}

fn is_letter(c: char) -> bool {
    c.is_alphanumeric() && !is_apostrophe(c)
}

fn fold(c: char, out: &mut String) {
    for lower in c.to_lowercase() {
        match lower {
            'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' | 'ā' => out.push('a'),
            'è' | 'é' | 'ê' | 'ë' | 'ē' => out.push('e'),
            'ì' | 'í' | 'î' | 'ï' | 'ī' => out.push('i'),
            'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'ø' | 'ō' => out.push('o'),
            'ù' | 'ú' | 'û' | 'ü' | 'ū' => out.push('u'),
            'ý' | 'ÿ' => out.push('y'),
            'ç' => out.push('c'),
            'ñ' => out.push('n'),
            'æ' => out.push_str("ae"),
            'œ' => out.push_str("oe"),
            'ß' => out.push_str("ss"),
            other => out.push(other),
        }
    }
}

/// The part of an English word its plural, -ed and -ing forms share: "recipes" and "recipe", "reducing" and
/// "reduce". The result need not be a word. Two words with the same stem are treated as near matches.
pub fn stem(term: &str) -> String {
    if term.len() < 3 || !term.bytes().all(|b| b.is_ascii_lowercase()) {
        return term.to_string();
    }
    let mut s = term.to_string();
    if s.len() > 3 {
        if s.ends_with("ies") {
            s.truncate(s.len() - 3);
            s.push('y');
        } else if s.ends_with('s') && !["ss", "us", "is"].iter().any(|end| s.ends_with(end)) {
            s.pop();
        }
    }
    for suffix in ["ing", "ed"] {
        let Some(root) = s.strip_suffix(suffix) else { continue };
        let enough = if root.ends_with('e') { root.len() >= 4 } else { root.len() >= 2 };
        if enough && root.bytes().any(is_vowel) {
            let mut root = root.to_string();
            if suffix == "ed" && root.ends_with('i') {
                root.pop();
                root.push('y');
            }
            let b = root.as_bytes();
            let last = b[b.len() - 1];
            if b.len() >= 4 && last == b[b.len() - 2] && !is_vowel(last) && !matches!(last, b'l' | b's' | b'z') {
                root.pop();
            }
            s = root;
        }
        break;
    }
    if s.len() > 2 && s.ends_with('e') {
        s.pop();
    }
    s
}

fn is_vowel(b: u8) -> bool {
    matches!(b, b'a' | b'e' | b'i' | b'o' | b'u' | b'y')
}

/// How many edits turn `a` into `b`, if no more than `max`: a letter added, dropped or changed, or two neighbours
/// swapped. Words of 64 letters or more are never near anything.
pub fn edits<T: PartialEq>(a: &[T], b: &[T], max: usize) -> Option<usize> {
    const MOST: usize = 64;
    if a.len().abs_diff(b.len()) > max || a.len() >= MOST || b.len() >= MOST {
        return None;
    }
    // Three rows of the table, reused: row i lives at rows[(i + 1) % 3].
    let mut rows = [[0u8; MOST]; 3];
    for (j, cell) in rows[1].iter_mut().enumerate().take(b.len() + 1) {
        *cell = j as u8;
    }
    for i in 1..=a.len() {
        let (back2, back1, row) = (&rows[(i + 2) % 3], &rows[i % 3], (i + 1) % 3);
        let mut next = [0u8; MOST];
        next[0] = i as u8;
        let mut least = i as u8;
        for j in 1..=b.len() {
            let change = if a[i - 1] == b[j - 1] { 0 } else { 1 };
            let mut cell = (back1[j] + 1).min(next[j - 1] + 1).min(back1[j - 1] + change);
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                cell = cell.min(back2[j - 2] + 1);
            }
            next[j] = cell;
            least = least.min(cell);
        }
        if least as usize > max {
            return None;
        }
        rows[row] = next;
    }
    let total = rows[(a.len() + 1) % 3][b.len()] as usize;
    (total >= 1 && total <= max).then_some(total)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn terms(text: &str) -> Vec<String> {
        words(text).into_iter().map(|w| w.term).collect()
    }

    #[test]
    fn splits_and_lowercases() {
        assert_eq!(terms("Hello World! Rust is great."), ["hello", "world", "rust", "is", "great"]);
    }

    #[test]
    fn keeps_common_words_and_single_letters() {
        assert_eq!(terms("I see the work"), ["i", "see", "the", "work"]);
    }

    #[test]
    fn drops_apostrophes_and_accents() {
        assert_eq!(terms("don't I\u{2019}ve Haleakalā Hawai\u{02BB}i 'quoted'"), ["dont", "ive", "haleakala", "hawaii", "quoted"]);
    }

    #[test]
    fn the_form_keeps_the_apostrophe_inside_a_word() {
        let forms: Vec<String> = words("Lov\u{2019}d o'er 'tis").into_iter().map(|w| w.form).collect();
        assert_eq!(forms, ["lov'd", "o'er", "tis"]);
    }

    #[test]
    fn records_where_each_word_sits() {
        let text = "tide, then tides";
        let found = words(text);
        assert_eq!(&text[found[2].start..found[2].end], "tides");
    }

    #[test]
    fn stems_join_the_forms_of_a_word() {
        for group in [
            vec!["recipe", "recipes"],
            vec!["curry", "currys", "curries"],
            vec!["reduce", "reduced", "reducing", "reduces"],
            vec!["run", "running", "runs"],
            vec!["try", "tried", "tries"],
            vec!["use", "used", "using"],
            vec!["need", "needs", "needed"],
            vec!["tell", "telling"],
            vec!["add", "added", "adding"],
        ] {
            let stems: Vec<String> = group.iter().map(|w| stem(w)).collect();
            assert!(stems.iter().all(|s| s == &stems[0]), "{group:?} gave {stems:?}");
        }
    }

    #[test]
    fn stems_leave_other_words_alone() {
        for word in ["king", "thing", "string", "bus", "this", "glass"] {
            assert_eq!(stem(word), word);
        }
    }

    #[test]
    fn counts_edits() {
        let count = |a: &str, b: &str, max| edits(&a.chars().collect::<Vec<_>>(), &b.chars().collect::<Vec<_>>(), max);
        assert_eq!(count("raineir", "rainier", 1), Some(1));
        assert_eq!(count("cocnut", "coconut", 1), Some(1));
        assert_eq!(count("ingrediants", "ingredients", 2), Some(1));
        assert_eq!(count("curry", "berry", 1), None);
    }
}
