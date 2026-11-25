//! What a corpus says about its own language, as tab-separated tables. Each line is a key and then its values;
//! a line starting with `#` is a comment. A value may end in `:weight`, from 0 to 100.
//!
//! - `forms`: a word, then the word it is a form of ("spake speak", "thou you"). A word mapped to itself is kept
//!   from the suffix rules ("honest honest").
//! - `suffixes`: an ending, then what it becomes before stemming ("eth s", "'d ed"). An empty value drops it.
//! - `synonyms`: a word, then words that say the same thing. They match at less than the word itself.
//! - `aliases`: a name in the corpus's references, then the other names it goes by ("Hamlet Ham").
//! - `stop`: words that add to a match but never make one.

use crate::fast::{Map as HashMap, Set as HashSet};

use crate::text::{plain, stem, words};

#[derive(Default, Clone, Debug)]
pub struct Tables {
    forms: HashMap<String, String>,
    /// Longest ending first, so "edst" is tried before "st".
    suffixes: Vec<(String, String)>,
    /// By the stem of the word, the stems that say the same thing and how strongly.
    synonyms: HashMap<String, Vec<(String, f32)>>,
    /// Each name and the other names it goes by, as written.
    pub aliases: Vec<(String, Vec<String>)>,
    stop: HashSet<String>,
    /// The tables as they were given, by name, so an index can carry them.
    pub sources: Vec<(String, String)>,
}

/// The table names a corpus may supply.
pub const NAMES: [&str; 5] = ["forms", "suffixes", "synonyms", "aliases", "stop"];

impl Tables {
    /// Add a table by name. Returns an error naming the line that could not be read.
    pub fn add(&mut self, name: &str, source: &str) -> Result<(), String> {
        if !NAMES.contains(&name) {
            return Err(format!("{name}: not a table name; use one of {}", NAMES.join(", ")));
        }
        self.sources.push((name.to_string(), source.to_string()));
        let rows = rows(source);
        match name {
            "forms" => {
                for (i, row) in rows {
                    let [key, base] = row.as_slice() else { return Err(format!("forms line {i}: want a word and its base")) };
                    self.forms.insert(key_form(key), key_form(base));
                }
            }
            "suffixes" => {
                for (i, row) in rows {
                    let (ending, becomes) = match row.as_slice() {
                        [ending] => (ending, ""),
                        [ending, becomes] => (ending, becomes.as_str()),
                        _ => return Err(format!("suffixes line {i}: want an ending and what it becomes")),
                    };
                    self.suffixes.push((ending.to_lowercase(), becomes.to_lowercase()));
                }
                self.suffixes.sort_by_key(|(ending, _)| std::cmp::Reverse(ending.len()));
            }
            "synonyms" => {
                // In this table a cell may hold several words, so "word<TAB>a:100 b:67" reads too.
                for (i, row) in rows {
                    let Some((key, values)) = row.split_first() else { continue };
                    let key = self.stem_of(&key_form(key));
                    let mut list = Vec::new();
                    for value in values.iter().flat_map(|cell| cell.split(' ')).filter(|v| !v.is_empty()) {
                        let (word, weight) = weighted(value).ok_or_else(|| format!("synonyms line {i}: bad weight in {value:?}"))?;
                        list.push((self.stem_of(&key_form(word)), weight));
                    }
                    self.synonyms.entry(key).or_default().extend(list);
                }
            }
            "aliases" => {
                for (_, row) in rows {
                    if let Some((name, others)) = row.split_first() {
                        self.aliases.push((name.clone(), others.to_vec()));
                    }
                }
            }
            _ => {
                for (_, row) in rows {
                    for word in row {
                        let stem = self.stem_of(&key_form(&word));
                        self.stop.insert(stem);
                    }
                }
            }
        }
        Ok(())
    }

    /// The stem a word is matched by, from its form (lower case, apostrophes kept as `'`).
    pub fn stem_of(&self, form: &str) -> String {
        let bare = |s: &str| s.replace('\'', "");
        if let Some(base) = self.forms.get(form).or_else(|| self.forms.get(&bare(form))) {
            return stem(&bare(base));
        }
        for (ending, becomes) in &self.suffixes {
            let Some(root) = form.strip_suffix(ending.as_str()) else { continue };
            let root = bare(root);
            if root.len() >= 3 && root.bytes().any(|b| b"aeiouy".contains(&b)) {
                return stem(&format!("{root}{becomes}"));
            }
        }
        stem(&bare(form))
    }

    /// Whether a word's stem only adds to matches.
    pub fn is_stop(&self, stem: &str) -> bool {
        self.stop.contains(stem)
    }

    pub fn synonyms(&self, stem: &str) -> &[(String, f32)] {
        self.synonyms.get(stem).map_or(&[], |v| v.as_slice())
    }
}

/// A table's lines as cells, numbered from 1, without blank lines and comments.
fn rows(source: &str) -> Vec<(usize, Vec<String>)> {
    source
        .lines()
        .enumerate()
        .filter(|(_, line)| !line.trim().is_empty() && !line.starts_with('#'))
        .map(|(i, line)| (i + 1, line.split('\t').map(|cell| cell.trim().to_string()).filter(|c| !c.is_empty()).collect()))
        .collect()
}

/// A word as a table key: folded the way text is, apostrophes kept.
fn key_form(word: &str) -> String {
    let found = words(word);
    match found.as_slice() {
        [one] => one.form.clone(),
        _ => plain(word),
    }
}

fn weighted(value: &str) -> Option<(&str, f32)> {
    match value.rsplit_once(':') {
        Some((word, weight)) => Some((word, weight.parse::<f32>().ok()? / 100.0)),
        None => Some((value, 1.0)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn early_modern() -> Tables {
        let mut t = Tables::default();
        t.add("forms", "hath\thave\nthou\tyou\nhonest\thonest\n").unwrap();
        t.add("suffixes", "eth\ts\n'd\ted\nest\n").unwrap();
        t.add("synonyms", "before\tere\noften\toft:80\n").unwrap();
        t.add("stop", "the\tyou\n").unwrap();
        t
    }

    #[test]
    fn forms_and_suffixes_join_old_and_new_words() {
        let t = early_modern();
        assert_eq!(t.stem_of("hath"), t.stem_of("have"));
        assert_eq!(t.stem_of("thou"), t.stem_of("you"));
        assert_eq!(t.stem_of("loveth"), t.stem_of("loves"));
        assert_eq!(t.stem_of("lov'd"), t.stem_of("loved"));
        assert_eq!(t.stem_of("knowest"), t.stem_of("know"));
        assert_eq!(t.stem_of("honest"), "honest");
    }

    #[test]
    fn a_short_root_keeps_its_ending() {
        let t = early_modern();
        assert_eq!(t.stem_of("best"), "best");
        assert_eq!(t.stem_of("teeth"), "teeth");
    }

    #[test]
    fn synonyms_carry_weights() {
        let t = early_modern();
        assert_eq!(t.synonyms(&t.stem_of("before")), [(t.stem_of("ere"), 1.0)]);
        assert_eq!(t.synonyms(&t.stem_of("often")), [(t.stem_of("oft"), 0.8)]);
    }

    #[test]
    fn stop_words_are_matched_by_stem() {
        let t = early_modern();
        assert!(t.is_stop(&t.stem_of("thou")));
        assert!(!t.is_stop(&t.stem_of("king")));
    }

    #[test]
    fn an_unknown_table_is_an_error() {
        assert!(Tables::default().add("colours", "a\tb").is_err());
    }
}
