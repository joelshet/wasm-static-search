//! A corpus as records: tab-separated text, one record per line, under a header line that names the columns.
//!
//! ```text
//! #ref	text	speaker	kind
//! Hamlet 3.1.56	To be, or not to be, that is the question:	Hamlet	verse
//! ```
//!
//! `ref` and `text` are required. `heading` and `anchor` are the heading a text sits under and its id. `boost` is a
//! number that multiplies the record's score by 1 + boost, so among records matching a query equally well the
//! better known comes first; it's 0 when missing. Every other column is a field: shown with results and usable as a filter (`speaker:hamlet`). `name*2` searches a
//! field with each word counting twice; `name^2` adds twice a word's rarity when the field holds it. `region+1`
//! counts a field's words once, but only for a record that the query's other words find, so a place's state can
//! narrow a search for its name without finding everything in the state. In a cell, `\t`, `\n` and `\\` stand
//! for a tab, a line break and a backslash.
//!
//! A ref is a name and then, after the last space, numbers joined by `.` or `:` ("Hamlet 3.1.56", "Psalms
//! 23:1"). Records sharing a name and every number but the last are neighbours, in file order. A ref without
//! numbers is a key, like a dictionary's headword.

use crate::tables::Tables;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Section {
    pub anchor: String,
    pub heading: String,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Field {
    pub name: String,
    /// How much each word counts when searched; 0 leaves the field out of the search.
    pub count: f32,
    /// Added, times a word's rarity, when the field holds the word.
    pub bonus: f32,
    /// Its words add to a record that the query's other words found, and find none themselves.
    pub adds: bool,
}

#[derive(Clone, Debug, Default)]
pub struct Record {
    pub reference: String,
    /// Multiplies the record's score by 1 + boost.
    pub boost: f32,
    /// One value per field of the corpus, in its order.
    pub fields: Vec<String>,
    pub sections: Vec<Section>,
}

#[derive(Clone, Debug, Default)]
pub struct Corpus {
    pub fields: Vec<Field>,
    pub records: Vec<Record>,
    pub tables: Tables,
}

/// The first line of an index file, then a part per line: a name and a byte count, those bytes, a line break.
const MAGIC: &str = "wasm-static-search index 1\n";

impl Corpus {
    /// Read records from TSV. Errors name the line.
    pub fn from_tsv(source: &str) -> Result<Corpus, String> {
        let mut lines = source.lines().enumerate().filter(|(_, l)| !l.is_empty());
        let Some((_, header)) = lines.next() else { return Ok(Corpus::default()) };
        let Some(header) = header.strip_prefix('#') else { return Err("line 1: the header must start with #".into()) };
        let mut corpus = Corpus::default();
        let (mut at_ref, mut at_text, mut at_heading, mut at_anchor, mut at_boost) = (None, None, None, None, None);
        let mut at_field = Vec::new();
        for (i, column) in header.split('\t').enumerate() {
            let (name, count, bonus, adds) = column_spec(column).ok_or_else(|| format!("line 1: can't read column {column:?}"))?;
            match name {
                "ref" => at_ref = Some(i),
                "text" => at_text = Some(i),
                "heading" => at_heading = Some(i),
                "anchor" => at_anchor = Some(i),
                "boost" => at_boost = Some(i),
                _ => {
                    at_field.push(i);
                    corpus.fields.push(Field { name: name.to_string(), count, bonus, adds });
                }
            }
        }
        let (Some(at_ref), Some(at_text)) = (at_ref, at_text) else { return Err("line 1: the header needs ref and text".into()) };
        for (i, line) in lines {
            if line.starts_with('#') {
                continue;
            }
            let mut cells: Vec<String> = line.split('\t').map(unescape).collect();
            if cells.len() <= at_ref.max(at_text) {
                return Err(format!("line {}: want at least {} cells", i + 1, at_ref.max(at_text) + 1));
            }
            let mut cell = |at: Option<usize>| at.and_then(|a| cells.get_mut(a)).map(std::mem::take).unwrap_or_default();
            let boost = cell(at_boost);
            let boost = if boost.is_empty() { 0.0 } else { boost.parse().map_err(|_| format!("line {}: boost {boost:?} is not a number", i + 1))? };
            corpus.records.push(Record {
                reference: cell(Some(at_ref)),
                boost,
                fields: at_field.iter().map(|&a| cell(Some(a))).collect(),
                sections: vec![Section { anchor: cell(at_anchor), heading: cell(at_heading), text: cell(Some(at_text)) }],
            });
        }
        Ok(corpus)
    }

    /// The records as TSV, one line per section. A boost column comes only when some record has a boost.
    pub fn to_tsv(&self) -> String {
        let boosted = self.records.iter().any(|r| r.boost != 0.0);
        let mut out = String::from("#ref\ttext\theading\tanchor");
        if boosted {
            out.push_str("\tboost");
        }
        for field in &self.fields {
            out.push('\t');
            out.push_str(&field.name);
            if field.count != 0.0 {
                out.push_str(&format!("{}{}", if field.adds { '+' } else { '*' }, field.count));
            }
            if field.bonus != 0.0 {
                out.push_str(&format!("^{}", field.bonus));
            }
        }
        out.push('\n');
        for record in &self.records {
            for section in &record.sections {
                let boost = record.boost.to_string();
                let mut cells = vec![&record.reference, &section.text, &section.heading, &section.anchor];
                if boosted {
                    cells.push(&boost);
                }
                cells.extend(&record.fields);
                out.push_str(&cells.iter().map(|c| escape(c)).collect::<Vec<_>>().join("\t"));
                out.push('\n');
            }
        }
        out
    }

    /// An index file: the records and the tables they came with, in one file.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = MAGIC.as_bytes().to_vec();
        let mut part = |name: &str, body: &str| {
            out.extend(format!("{name} {}\n", body.len()).as_bytes());
            out.extend(body.as_bytes());
            out.push(b'\n');
        };
        part("records", &self.to_tsv());
        for (name, source) in &self.tables.sources {
            part(name, source);
        }
        out
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Corpus, String> {
        let text = std::str::from_utf8(bytes).map_err(|_| "not an index: not UTF-8")?;
        let mut rest = text.strip_prefix(MAGIC).ok_or("not an index: wrong first line")?;
        let mut corpus = None;
        let mut tables = Tables::default();
        while !rest.is_empty() {
            let (head, after) = rest.split_once('\n').ok_or("index cut short")?;
            let (name, len) = head.split_once(' ').ok_or("index part has no length")?;
            let len: usize = len.parse().map_err(|_| "index part length is not a number")?;
            let body = after.get(..len).ok_or("index cut short")?;
            rest = after[len..].strip_prefix('\n').ok_or("index part does not end in a line break")?;
            if name == "records" {
                corpus = Some(Corpus::from_tsv(body)?);
            } else {
                tables.add(name, body)?;
            }
        }
        let mut corpus = corpus.ok_or("index has no records")?;
        corpus.tables = tables;
        Ok(corpus)
    }
}

/// A column's name, count, bonus and whether its words only add: "title", "title*2", "title^2", "title*1^2",
/// "region+1".
fn column_spec(column: &str) -> Option<(&str, f32, f32, bool)> {
    let (rest, bonus) = match column.split_once('^') {
        Some((rest, b)) => (rest, b.parse().ok()?),
        None => (column, 0.0),
    };
    let (name, count, adds) = match rest.split_once(['*', '+']) {
        Some((name, c)) => (name, c.parse().ok()?, rest.as_bytes()[name.len()] == b'+'),
        None => (rest, 0.0, false),
    };
    let name = name.trim();
    (!name.is_empty()).then_some((name, count, bonus, adds))
}

fn unescape(cell: &str) -> String {
    if !cell.contains('\\') {
        return cell.to_string();
    }
    let mut out = String::with_capacity(cell.len());
    let mut chars = cell.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('t') => out.push('\t'),
            Some('n') => out.push('\n'),
            Some(other) => out.push(other),
            None => out.push('\\'),
        }
    }
    out
}

pub fn escape(cell: &str) -> String {
    cell.replace('\\', "\\\\").replace('\t', "\\t").replace('\n', "\\n")
}

/// A ref's name and its numbers: "Hamlet 3.1.56" is ("Hamlet", [3, 1, 56]); "cooper" is ("cooper", []).
pub fn split_ref(reference: &str) -> (&str, Vec<u32>) {
    let reference = reference.trim();
    let (name, last) = reference.rsplit_once(' ').unwrap_or(("", reference));
    match numbers(last) {
        Some(found) if !name.trim().is_empty() => (name.trim(), found),
        _ => (reference, Vec::new()),
    }
}

/// "3.1.56" or "23:1" as numbers; None for anything else.
pub fn numbers(text: &str) -> Option<Vec<u32>> {
    if text.is_empty() || !text.starts_with(|c: char| c.is_ascii_digit()) || !text.ends_with(|c: char| c.is_ascii_digit()) {
        return None;
    }
    text.split(['.', ':']).map(|n| n.parse().ok()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const PLAY: &str = "#ref\ttext\tspeaker\tkind\n\
        Hamlet 3.1.56\tTo be, or not to be, that is the question:\tHamlet\tverse\n\
        Hamlet 3.1.57\tWhether 'tis nobler\\tin the mind\tHamlet\tverse\n";

    #[test]
    fn reads_records_and_fields() {
        let corpus = Corpus::from_tsv(PLAY).unwrap();
        assert_eq!(corpus.records.len(), 2);
        assert_eq!(corpus.fields.iter().map(|f| f.name.as_str()).collect::<Vec<_>>(), ["speaker", "kind"]);
        assert_eq!(corpus.records[0].fields, ["Hamlet", "verse"]);
        assert_eq!(corpus.records[1].sections[0].text, "Whether 'tis nobler\tin the mind");
    }

    #[test]
    fn reads_weights_from_the_header() {
        let corpus = Corpus::from_tsv("#ref\ttext\ttitle*1^2\tdescription*2\tregion+1\n/a/\tbody\tA\tAn a\tOregon\n").unwrap();
        assert_eq!(corpus.fields[0], Field { name: "title".into(), count: 1.0, bonus: 2.0, adds: false });
        assert_eq!(corpus.fields[1], Field { name: "description".into(), count: 2.0, bonus: 0.0, adds: false });
        assert_eq!(corpus.fields[2], Field { name: "region".into(), count: 1.0, bonus: 0.0, adds: true });
        assert_eq!(Corpus::from_bytes(&corpus.to_bytes()).unwrap().fields, corpus.fields);
    }

    #[test]
    fn reads_a_boost_and_keeps_it_out_of_the_fields() {
        let corpus = Corpus::from_tsv("#ref\ttext\tboost\tkind\nParis\tParis\t0.5\tcity\nParis\tParis\t\ttown\n").unwrap();
        assert_eq!(corpus.fields.len(), 1);
        assert_eq!((corpus.records[0].boost, corpus.records[1].boost), (0.5, 0.0));
        assert!(Corpus::from_tsv("#ref\ttext\tboost\nParis\tParis\tvery\n").is_err());
        let back = Corpus::from_bytes(&corpus.to_bytes()).unwrap();
        assert_eq!(back.records[0].boost, 0.5);
        assert!(!Corpus::from_tsv(PLAY).unwrap().to_tsv().contains("boost"));
    }

    #[test]
    fn a_header_without_text_is_an_error() {
        assert!(Corpus::from_tsv("#ref\tspeaker\nHamlet 1.1.1\tBarnardo\n").is_err());
        assert!(Corpus::from_tsv("ref\ttext\n").is_err());
    }

    #[test]
    fn an_index_file_round_trips() {
        let mut corpus = Corpus::from_tsv(PLAY).unwrap();
        corpus.tables.add("forms", "hath\thave\n").unwrap();
        let back = Corpus::from_bytes(&corpus.to_bytes()).unwrap();
        assert_eq!(back.records.len(), 2);
        assert_eq!(back.records[1].sections, corpus.records[1].sections);
        assert_eq!(back.tables.sources, corpus.tables.sources);
    }

    #[test]
    fn splits_refs() {
        assert_eq!(split_ref("Hamlet 3.1.56"), ("Hamlet", vec![3, 1, 56]));
        assert_eq!(split_ref("Henry IV, Part 1 5.4.110"), ("Henry IV, Part 1", vec![5, 4, 110]));
        assert_eq!(split_ref("Psalms 23:1"), ("Psalms", vec![23, 1]));
        assert_eq!(split_ref("cooper"), ("cooper", vec![]));
        assert_eq!(split_ref("/recipes/panang-curry/"), ("/recipes/panang-curry/", vec![]));
        assert_eq!(split_ref("1984"), ("1984", vec![]));
    }
}
