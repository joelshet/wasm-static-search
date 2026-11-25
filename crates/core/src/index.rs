use std::collections::{HashMap, HashSet};

use crate::bm25;
use crate::records::{numbers, split_ref, Corpus, Section};
use crate::fast::{Map, Set};
use crate::tables::Tables;
use crate::text::{each_word, edits, plain, words};

/// One word of a record's text: which term it is and where it sits.
#[derive(Clone, Copy, Debug)]
struct Placed {
    term: u32,
    section: u32,
    start: u32,
    end: u32,
}

#[derive(Debug)]
struct Doc {
    /// Every word of the sections' text, in reading order.
    words: Vec<Placed>,
    /// Where each section's words begin in `words`.
    starts: Vec<u32>,
    /// Terms in a field with a bonus, and the bonus.
    bonus: Vec<(u32, f32)>,
    /// The words of searched fields in order, each field ending in `u32::MAX`, for word order.
    field_words: Vec<u32>,
    length: f32,
    /// The ref's name, as an index into `names`, and its numbers.
    name: u32,
    numbers: Vec<u32>,
    /// Records next to each other with the same group are neighbours.
    group: u32,
}

#[derive(Clone, Copy, Debug)]
struct Posting {
    doc: u32,
    /// How often the record says the term, with headings and weighted fields counted extra.
    count: f32,
}

/// A corpus arranged for search.
pub struct Index {
    pub corpus: Corpus,
    docs: Vec<Doc>,
    /// Every distinct term, sorted, so a prefix is a run of neighbours.
    terms: Vec<String>,
    /// The records holding each term, in step with `terms`.
    postings: Vec<Vec<Posting>>,
    /// The records holding each term in a field whose words only add to a match: those of term `t` are
    /// `add_postings[add_starts[t]..add_starts[t + 1]]`. Both are empty when no field only adds.
    add_starts: Vec<u32>,
    add_postings: Vec<Posting>,
    /// The terms sharing each stem.
    stems: Map<String, Vec<u32>>,
    /// Which letters each term holds, a bit each, to rule out typo matches before counting edits.
    letters: Vec<u32>,
    avg_length: f32,
    /// Each ref name as the records write it.
    names: Vec<String>,
    /// A name or alias, as plain words, and the name it stands for.
    name_ids: Map<String, u32>,
    /// The records under each name, in order.
    name_docs: Vec<Vec<u32>>,
}

const HEADING_COUNT: f32 = 2.0;
/// A match on another form of the word ("recipes" for "recipe"), against 1.0 for the word itself.
const STEM_WEIGHT: f32 = 0.9;
/// A match one or two edits away, tried only when nothing nearer exists.
const TYPO_WEIGHTS: [f32; 2] = [0.5, 0.35];
/// A match through the corpus's synonyms, times the synonym's own weight.
const SYNONYM_WEIGHT: f32 = 0.6;
/// The most words a snippet's matches may be spread over.
const WINDOW: usize = 16;
const SNIPPET_BYTES: usize = 150;
/// The most postings a word still being typed may draw in through the words it begins.
const COMPLETION_BUDGET: usize = 40_000;
/// How many of the best records get the closer look: word order, and passages.
const CLOSE: usize = 300;
/// A passage remembered across neighbouring records is one result, up to this many records long.
const MAX_RUN: usize = 4;
/// What each record past the first costs a passage, so one record holding the query still wins.
const RUN_COST: f32 = 0.8;
/// How much a record gains when it keeps every pair of the query's neighbouring words side by side, in order.
const ORDER_GAIN: f32 = 1.0;
/// Up to this many words the gain is whole; past it, it shrinks in step with length, since in a long text
/// words land side by side by chance.
const ORDER_WORDS: f32 = 40.0;

/// One record of a result: where to go and the words found there.
#[derive(Debug, Clone)]
pub struct Shown {
    pub doc: usize,
    /// The heading the best match sits under, and its id. Empty when the match is above the first heading.
    pub heading: String,
    pub anchor: String,
    /// The text around the best match, in pieces. A piece marked true is a word that matched.
    pub snippet: Vec<(String, bool)>,
    /// The first matched word and the few after it, exactly as the text has them, for finding the spot.
    pub find: String,
}

/// One result: a record, or a passage of neighbouring records.
#[derive(Debug, Clone)]
pub struct Hit {
    pub score: f32,
    pub shown: Vec<Shown>,
}

/// A word of the query, and the index terms that stand for it, each with how good a match it is.
struct Asked {
    stop: bool,
    matches: Vec<(u32, f32)>,
}

enum Filter {
    Names(HashSet<u32>),
    /// A field, and words each of which must begin a word of it.
    Field(usize, Vec<String>),
}

struct Query {
    asked: Vec<Asked>,
    /// The query's words in order, as indexes into `asked`, repeats kept.
    order: Vec<usize>,
    /// Words from a quoted phrase, which a result must hold.
    required: Vec<usize>,
    filters: Vec<Filter>,
    /// A ref to go straight to: a name, numbers, and the last number of a range.
    reference: Option<(u32, Vec<u32>, Option<u32>)>,
}

/// A candidate result: its first and last record, how many of the query's words it holds, and its score.
#[derive(Clone, Copy)]
struct Candidate {
    first: u32,
    last: u32,
    had: u32,
    score: f32,
}

/// Terms as they turn up while building, and the counts for the record being read.
struct Terms<'t> {
    ids: Map<String, u32>,
    /// The stem of each term, from the first form it turned up in.
    stems: Vec<String>,
    postings: Vec<Vec<Posting>>,
    tables: &'t Tables,
    /// For each term, the record it was last counted in (plus one) and where in `count` that count sits.
    last: Vec<(u32, u32)>,
    count: Vec<(u32, f32)>,
    record: u32,
}

impl<'t> Terms<'t> {
    fn new(tables: &'t Tables) -> Self {
        Terms { ids: Map::default(), stems: Vec::new(), postings: Vec::new(), tables, last: Vec::new(), count: Vec::new(), record: 0 }
    }

    fn id(&mut self, term: &str, form: &str) -> u32 {
        if let Some(&id) = self.ids.get(term) {
            return id;
        }
        let id = self.ids.len() as u32;
        self.ids.insert(term.to_string(), id);
        self.stems.push(self.tables.stem_of(form));
        self.postings.push(Vec::new());
        self.last.push((0, 0));
        id
    }

    fn next_record(&mut self) {
        self.record += 1;
        self.count.clear();
    }

    fn bump(&mut self, term: u32, by: f32) {
        let (record, at) = &mut self.last[term as usize];
        if *record == self.record {
            self.count[*at as usize].1 += by;
        } else {
            *record = self.record;
            *at = self.count.len() as u32;
            self.count.push((term, by));
        }
    }

    fn post(&mut self, doc: u32) {
        for &(term, count) in &self.count {
            if count > 0.0 {
                self.postings[term as usize].push(Posting { doc, count });
            }
        }
    }
}

impl Index {
    pub fn new(mut corpus: Corpus) -> Index {
        for record in &mut corpus.records {
            for section in &mut record.sections {
                squeeze(&mut section.heading);
                squeeze(&mut section.text);
            }
        }
        let mut terms = Terms::new(&corpus.tables);
        let mut docs: Vec<Doc> = Vec::with_capacity(corpus.records.len());
        let mut names: Vec<String> = Vec::new();
        let mut name_ids: Map<String, u32> = Map::default();
        let mut name_docs: Vec<Vec<u32>> = Vec::new();
        let mut groups = 0u32;

        for (d, record) in corpus.records.iter().enumerate() {
            terms.next_record();
            let mut placed = Vec::with_capacity(record.sections.iter().map(|s| s.text.len() / 5 + 1).sum());
            let mut starts = Vec::new();
            let mut bonus = Vec::new();
            let mut length = 0.0;
            for (i, section) in record.sections.iter().enumerate() {
                starts.push(placed.len() as u32);
                each_word(&section.heading, |term, form, _, _| {
                    let id = terms.id(term, form);
                    terms.bump(id, HEADING_COUNT);
                    length += 1.0;
                });
                each_word(&section.text, |term, form, start, end| {
                    let id = terms.id(term, form);
                    terms.bump(id, 1.0);
                    placed.push(Placed { term: id, section: i as u32, start: start as u32, end: end as u32 });
                });
            }
            length += placed.len() as f32;
            let mut field_words = Vec::new();
            for (field, value) in corpus.fields.iter().zip(&record.fields) {
                if field.adds || field.count == 0.0 && field.bonus == 0.0 {
                    continue;
                }
                each_word(value, |term, form, _, _| {
                    let id = terms.id(term, form);
                    terms.bump(id, field.count);
                    field_words.push(id);
                    length += 1.0;
                    if field.bonus > 0.0 && !bonus.iter().any(|&(t, _)| t == id) {
                        bonus.push((id, field.bonus));
                    }
                });
                field_words.push(u32::MAX);
            }
            terms.post(d as u32);

            let (name, numbers) = split_ref(&record.reference);
            // Records under one name usually come together, so most need no lookup.
            let name = match docs.last() {
                Some(prev) if names[prev.name as usize] == name => prev.name,
                _ => *name_ids.entry(plain(name)).or_insert_with(|| {
                    names.push(name.to_string());
                    name_docs.push(Vec::new());
                    (names.len() - 1) as u32
                }),
            };
            name_docs[name as usize].push(d as u32);
            let same_group = docs.last().is_some_and(|prev| {
                prev.name == name && !numbers.is_empty() && prev.numbers.len() == numbers.len() && prev.numbers[..numbers.len() - 1] == numbers[..numbers.len() - 1]
            });
            if !same_group {
                groups += 1;
            }
            docs.push(Doc { words: placed, starts, bonus, field_words, length, name, numbers, group: groups });
        }
        // Fields whose words only add keep their postings apart, so those words can't find a record.
        let mut added: Vec<(u32, u32, f32)> = Vec::new();
        let adding: Vec<(usize, f32)> = corpus.fields.iter().enumerate().filter(|(_, f)| f.adds && f.count > 0.0).map(|(i, f)| (i, f.count)).collect();
        let mut counts: Vec<(u32, f32)> = Vec::new();
        for (d, record) in corpus.records.iter().enumerate().filter(|_| !adding.is_empty()) {
            for &(field, by) in &adding {
                each_word(&record.fields[field], |term, form, _, _| {
                    let id = terms.id(term, form);
                    // A few words, so a scan beats a lookup.
                    match counts.iter_mut().find(|(t, _)| *t == id) {
                        Some((_, count)) => *count += by,
                        None => counts.push((id, by)),
                    }
                });
            }
            added.extend(counts.drain(..).map(|(term, count)| (term, d as u32, count)));
        }
        let Terms { ids, stems: mut first_stems, postings, .. } = terms;

        // Terms were numbered as they turned up. Renumber them in sorted order.
        let mut sorted: Vec<(String, u32)> = ids.into_iter().collect();
        sorted.sort();
        let mut renumbered = vec![0u32; sorted.len()];
        for (new, (_, old)) in sorted.iter().enumerate() {
            renumbered[*old as usize] = new as u32;
        }
        let mut stems: Map<String, Vec<u32>> = Map::default();
        let mut by_new: Vec<Vec<Posting>> = vec![Vec::new(); sorted.len()];
        for (old, list) in postings.into_iter().enumerate() {
            let new = renumbered[old];
            by_new[new as usize] = list;
            stems.entry(std::mem::take(&mut first_stems[old])).or_default().push(new);
        }
        for list in stems.values_mut() {
            list.sort();
        }
        for doc in &mut docs {
            for word in &mut doc.words {
                word.term = renumbered[word.term as usize];
            }
            for (term, _) in &mut doc.bonus {
                *term = renumbered[*term as usize];
            }
            for term in doc.field_words.iter_mut().filter(|t| **t != u32::MAX) {
                *term = renumbered[*term as usize];
            }
        }
        // Laid out by term, each term's records staying in order.
        let (mut add_starts, mut add_postings) = (Vec::new(), Vec::new());
        if !added.is_empty() {
            add_starts = vec![0u32; sorted.len() + 1];
            for &(term, _, _) in &added {
                add_starts[renumbered[term as usize] as usize + 1] += 1;
            }
            for t in 1..add_starts.len() {
                add_starts[t] += add_starts[t - 1];
            }
            let mut next = add_starts.clone();
            add_postings = vec![Posting { doc: 0, count: 0.0 }; added.len()];
            for (term, doc, count) in added {
                let at = &mut next[renumbered[term as usize] as usize];
                add_postings[*at as usize] = Posting { doc, count };
                *at += 1;
            }
        }
        let terms: Vec<String> = sorted.into_iter().map(|(term, _)| term).collect();
        let letters = terms.iter().map(|t| letter_set(t)).collect();

        for (name, others) in &corpus.tables.aliases {
            if let Some(&id) = name_ids.get(&plain(name)) {
                for other in others {
                    name_ids.entry(plain(other)).or_insert(id);
                }
            }
        }

        let total: f32 = docs.iter().map(|d| d.length).sum();
        let avg_length = if docs.is_empty() { 0.0 } else { (total / docs.len() as f32).max(1.0) };
        Index { corpus, docs, terms, postings: by_new, add_starts, add_postings, stems, letters, avg_length, names, name_ids, name_docs }
    }

    pub fn len(&self) -> usize {
        self.docs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.docs.is_empty()
    }

    /// The record's ref, as written.
    pub fn reference(&self, doc: usize) -> &str {
        &self.corpus.records[doc].reference
    }

    /// A ref for a run of records: "Hamlet 3.1.56-57" when they share every number but the last.
    pub fn span(&self, first: usize, last: usize) -> String {
        let (a, b) = (&self.docs[first], &self.docs[last]);
        if first == last || a.numbers == b.numbers {
            return self.reference(first).to_string();
        }
        if a.group == b.group {
            return format!("{}-{}", self.reference(first), b.numbers.last().unwrap());
        }
        format!("{} to {}", self.reference(first), self.reference(last))
    }

    /// Rank the records for a query. A result holding more of the query's words always comes before one holding
    /// fewer. The last word is taken as still being typed unless the query ends in a space.
    pub fn search(&self, query: &str, max_results: usize) -> Vec<Hit> {
        let query = self.parse(query);
        // A ref with numbers is all the result there is. A key comes first, and the text search follows it.
        let mut hits: Vec<Hit> = Vec::new();
        if let Some((name, numbers, until)) = &query.reference {
            hits.extend(self.go_to(*name, numbers, *until));
            if !numbers.is_empty() {
                return hits;
            }
        }
        if query.asked.is_empty() {
            return hits;
        }
        let n = query.asked.len();
        let all_stop = query.asked.iter().all(|a| a.stop);
        let content: Vec<bool> = query.asked.iter().map(|a| all_stop || !a.stop).collect();

        // Each record's best score for each word of the query, a row of `n` in `rows`.
        const NONE: u32 = u32::MAX;
        let mut row_of: Vec<u32> = vec![NONE; self.docs.len()];
        let mut touched: Vec<u32> = Vec::new();
        let mut rows: Vec<f32> = Vec::new();
        // Content words first. A stop word only adds to a record that one of them already found, since a record
        // holding nothing but stop words is no result.
        let mut order: Vec<usize> = (0..n).collect();
        order.sort_by_key(|&i| !content[i]);
        for i in order {
            let asked = &query.asked[i];
            for &(term, weight) in &asked.matches {
                let list = &self.postings[term as usize];
                let rarity = bm25::idf(self.docs.len(), list.len());
                for posting in list {
                    if !content[i] && row_of[posting.doc as usize] == NONE {
                        continue;
                    }
                    let doc = &self.docs[posting.doc as usize];
                    let mut score = rarity * bm25::saturation(posting.count, doc.length, self.avg_length);
                    if let Some(&(_, bonus)) = doc.bonus.iter().find(|&&(t, _)| t == term) {
                        score += rarity * bonus;
                    }
                    let slot = &mut row_of[posting.doc as usize];
                    if *slot == NONE {
                        *slot = rows.len() as u32;
                        rows.resize(rows.len() + n, 0.0);
                        touched.push(posting.doc);
                    }
                    let cell = &mut rows[*slot as usize + i];
                    *cell = cell.max(score * weight);
                }
            }
        }
        // A word in a field that only adds, like a place's state, counts for a record the other words found. With
        // one word there are no others.
        if n > 1 && !self.add_postings.is_empty() {
            for (i, asked) in query.asked.iter().enumerate() {
                for &(term, weight) in &asked.matches {
                    let list = match self.add_starts.get(term as usize..term as usize + 2) {
                        Some(&[from, to]) if from < to => &self.add_postings[from as usize..to as usize],
                        _ => continue,
                    };
                    let rarity = bm25::idf(self.docs.len(), list.len());
                    let mut add = |posting: &Posting| {
                        let at = row_of[posting.doc as usize];
                        if at == NONE {
                            return;
                        }
                        let doc = &self.docs[posting.doc as usize];
                        let score = rarity * bm25::saturation(posting.count, doc.length, self.avg_length);
                        let cell = &mut rows[at as usize + i];
                        *cell = cell.max(score * weight);
                    };
                    // "america" is in the region of most places, so past a point it's quicker to look up each
                    // record found so far. Postings are in record order.
                    if touched.len() * 16 < list.len() {
                        for &doc in &touched {
                            if let Ok(k) = list.binary_search_by_key(&doc, |p| p.doc) {
                                add(&list[k]);
                            }
                        }
                    } else {
                        list.iter().for_each(&mut add);
                    }
                }
            }
        }
        let row = |doc: u32| {
            let at = row_of[doc as usize];
            (at != NONE).then(|| &rows[at as usize..at as usize + n])
        };
        let had = |cells: &[f32]| cells.iter().zip(&content).filter(|(s, c)| **c && **s > 0.0).count() as u32;
        let holds_required = |cells: &[f32]| query.required.iter().all(|&i| cells[i] > 0.0);

        let mut singles: Vec<Candidate> = Vec::new();
        for &doc in &touched {
            let cells = row(doc).unwrap();
            let had = had(cells);
            if had == 0 || !self.passes(doc as usize, &query.filters) {
                continue;
            }
            let boost = 1.0 + self.corpus.records[doc as usize].boost;
            singles.push(Candidate { first: doc, last: doc, had, score: cells.iter().sum::<f32>() * boost });
        }
        let best_first = |a: &Candidate, b: &Candidate| b.had.cmp(&a.had).then(b.score.total_cmp(&a.score)).then(a.first.cmp(&b.first));
        // Past the closer look, only enough to fill the results after passages swallow some records.
        let keep = CLOSE + MAX_RUN * max_results;
        if singles.len() > keep {
            // How many words a candidate holds sorts first, so the levels that fill `keep` are all that can rank.
            let mut at = vec![0usize; query.asked.len() + 1];
            for c in &singles {
                at[c.had as usize] += 1;
            }
            let (mut level, mut total) = (query.asked.len(), 0);
            while level > 1 && total + at[level] < keep {
                total += at[level];
                level -= 1;
            }
            singles.retain(|c| c.had as usize >= level);
        }
        singles.sort_by(best_first);
        singles.truncate(keep);

        // The closer look: word order within each of the best records, and passages around them.
        let wanted = wanted_terms(&query);
        let rarity: Vec<f32> = query
            .asked
            .iter()
            .map(|a| a.matches.iter().map(|&(t, _)| bm25::idf(self.docs.len(), self.postings[t as usize].len())).fold(0.0, f32::max))
            .collect();
        let pairs = pairs(&query.order, &rarity);
        let mut found: Vec<Candidate> = Vec::new();
        for (rank, single) in singles.iter().enumerate() {
            let mut single = *single;
            if rank >= CLOSE {
                if query.required.is_empty() || holds_required(row(single.first).unwrap()) {
                    found.push(single);
                }
                continue;
            }
            let cells = row(single.first).unwrap();
            if holds_required(cells) {
                single.score *= 1.0 + ORDER_GAIN * self.in_order(single.first, single.first, &wanted, &pairs);
                found.push(single);
            }
            if let Some(passage) = self.passage(single, n, &row, &had, &holds_required, &wanted, &pairs) {
                found.push(passage);
            }
        }
        found.sort_by(best_first);

        let marks = marks(&query);
        let mut taken: Set<u32> = hits.iter().flat_map(|h| h.shown.iter().map(|s| s.doc as u32)).collect();
        for c in found {
            if hits.len() == max_results {
                break;
            }
            if (c.first..=c.last).any(|d| taken.contains(&d)) {
                continue;
            }
            taken.extend(c.first..=c.last);
            let shown = (c.first..=c.last).map(|d| self.show(d as usize, &query, &marks)).collect();
            hits.push(Hit { score: c.score, shown });
        }
        hits
    }

    /// The best passage of neighbouring records around one record, when it holds more of the query than any of
    /// its records alone.
    #[allow(clippy::too_many_arguments)]
    fn passage<'a>(
        &self,
        seed: Candidate,
        n: usize,
        row: &impl Fn(u32) -> Option<&'a [f32]>,
        had: &impl Fn(&[f32]) -> u32,
        holds_required: &impl Fn(&[f32]) -> bool,
        wanted: &Map<u32, u64>,
        pairs: &[(usize, usize, f32)],
    ) -> Option<Candidate> {
        let group = self.docs[seed.first as usize].group;
        let mut best: Option<Candidate> = None;
        let s = seed.first as usize;
        let lo = s.saturating_sub(MAX_RUN - 1);
        for first in lo..=s {
            if self.docs[first].group != group || row(first as u32).is_none() {
                continue;
            }
            let mut union = vec![0f32; n];
            let mut most_alone = 0;
            for last in first..(first + MAX_RUN).min(self.docs.len()) {
                if self.docs[last].group != group {
                    break;
                }
                if let Some(cells) = row(last as u32) {
                    for (u, c) in union.iter_mut().zip(cells) {
                        *u = u.max(*c);
                    }
                    most_alone = most_alone.max(had(cells));
                }
                if last < s || last == first || row(last as u32).is_none_or(|cells| had(cells) == 0) {
                    continue;
                }
                let got = had(&union);
                if got <= most_alone || !holds_required(&union) {
                    continue;
                }
                let order = self.in_order(first as u32, last as u32, wanted, pairs);
                let boost = 1.0 + (first..=last).map(|d| self.corpus.records[d].boost).fold(0.0, f32::max);
                let score = union.iter().sum::<f32>() * RUN_COST.powi((last - first) as i32) * (1.0 + ORDER_GAIN * order) * boost;
                let c = Candidate { first: first as u32, last: last as u32, had: got, score };
                if best.is_none_or(|b| (c.had, c.score) > (b.had, b.score)) {
                    best = Some(c);
                }
            }
        }
        best
    }

    /// The share of the query's pairs of neighbouring words that these records hold side by side, in order, each
    /// pair counting by its rarer word's rarity: common pairs like "you have" turn up side by side by chance.
    fn in_order(&self, first: u32, last: u32, wanted: &Map<u32, u64>, pairs: &[(usize, usize, f32)]) -> f32 {
        if pairs.is_empty() {
            return 0.0;
        }
        let mut seen = vec![false; pairs.len()];
        let mut before = 0u64;
        let mut step = |term: u32, before: &mut u64| {
            let now = if term == u32::MAX { 0 } else { wanted.get(&term).copied().unwrap_or(0) };
            for (k, &(a, b, _)) in pairs.iter().enumerate() {
                if *before & (1 << a) != 0 && now & (1 << b) != 0 {
                    seen[k] = true;
                }
            }
            *before = now;
        };
        for d in first..=last {
            for word in &self.docs[d as usize].words {
                step(word.term, &mut before);
            }
        }
        // Fields such as a title count too, each on its own.
        for d in first..=last {
            before = 0;
            for &term in &self.docs[d as usize].field_words {
                step(term, &mut before);
            }
        }
        let total: f32 = pairs.iter().map(|p| p.2).sum();
        if total <= 0.0 {
            return 0.0;
        }
        let length: usize = (first..=last).map(|d| self.docs[d as usize].words.len()).sum();
        let share = pairs.iter().zip(&seen).filter(|(_, &s)| s).map(|(p, _)| p.2).sum::<f32>() / total;
        share * (ORDER_WORDS / length.max(1) as f32).min(1.0)
    }

    fn passes(&self, doc: usize, filters: &[Filter]) -> bool {
        filters.iter().all(|filter| match filter {
            Filter::Names(names) => names.contains(&self.docs[doc].name),
            Filter::Field(field, wanted) => {
                let have = plain(&self.corpus.records[doc].fields[*field]);
                let have: Vec<&str> = have.split(' ').collect();
                wanted.iter().all(|w| have.iter().any(|h| h.starts_with(w.as_str())))
            }
        })
    }

    /// The records a ref names: every record at exactly those numbers, or the first under a shorter prefix, or a
    /// range.
    fn go_to(&self, name: u32, numbers: &[u32], until: Option<u32>) -> Option<Hit> {
        let docs = &self.name_docs[name as usize];
        let starts = |d: &&u32| self.docs[**d as usize].numbers.starts_with(numbers);
        let picked: Vec<u32> = match until {
            Some(until) => {
                let (head, last) = numbers.split_at(numbers.len() - 1);
                docs.iter()
                    .filter(|&&d| {
                        let at = &self.docs[d as usize].numbers;
                        at.len() == numbers.len() && at.starts_with(head) && (last[0]..=until).contains(at.last().unwrap())
                    })
                    .copied()
                    .take(50)
                    .collect()
            }
            None => {
                let mut exact: Vec<u32> = docs.iter().filter(|&&d| self.docs[d as usize].numbers == numbers).copied().collect();
                // Every record of a key matches it equally well, so the boosted ones come first. A key has few
                // records, and an insertion sort is far smaller in WebAssembly than the standard library's.
                if numbers.is_empty() {
                    let boost = |d: u32| self.corpus.records[d as usize].boost;
                    for i in 1..exact.len() {
                        let mut j = i;
                        while j > 0 && boost(exact[j - 1]) < boost(exact[j]) {
                            exact.swap(j - 1, j);
                            j -= 1;
                        }
                    }
                }
                if exact.is_empty() { docs.iter().find(starts).copied().into_iter().collect() } else { exact }
            }
        };
        if picked.is_empty() {
            return None;
        }
        let empty = Query { asked: Vec::new(), order: Vec::new(), required: Vec::new(), filters: Vec::new(), reference: None };
        Some(Hit { score: 0.0, shown: picked.into_iter().map(|d| self.show(d as usize, &empty, &Map::default())).collect() })
    }

    fn parse(&self, text: &str) -> Query {
        let mut phrases: Vec<String> = Vec::new();
        let mut rest = String::new();
        let mut chars = text.chars();
        while let Some(c) = chars.next() {
            if matches!(c, '"' | '\u{201C}' | '\u{201D}') {
                let phrase: String = chars.by_ref().take_while(|&c| !matches!(c, '"' | '\u{201C}' | '\u{201D}')).collect();
                phrases.push(phrase);
                rest.push(' ');
            } else {
                rest.push(c);
            }
        }
        let mut filters = Vec::new();
        let mut kept: Vec<&str> = Vec::new();
        for token in rest.split_whitespace() {
            match token.split_once(':') {
                Some((key, value)) if !value.is_empty() && key.chars().all(char::is_alphabetic) && !key.is_empty() => {
                    if let Some(filter) = self.filter(key, value) {
                        filters.push(filter);
                    } else {
                        kept.push(token);
                    }
                }
                _ => kept.push(token),
            }
        }
        let plain_text = kept.join(" ");
        let mut query = Query { asked: Vec::new(), order: Vec::new(), required: Vec::new(), filters, reference: None };
        if phrases.is_empty() {
            query.reference = self.reference_in(&plain_text);
            if query.reference.as_ref().is_some_and(|(_, numbers, _)| !numbers.is_empty()) {
                return query;
            }
        }
        let open = !text.ends_with(char::is_whitespace) && !text.ends_with(['"', '\u{201D}']);
        let mut seen: HashMap<String, usize> = HashMap::new();
        let free = words(&plain_text);
        let last = free.len().saturating_sub(1);
        let mut add = |word: crate::text::Word, typing: bool, required: bool, query: &mut Query| {
            let i = *seen.entry(word.term.clone()).or_insert_with(|| {
                let stem = self.corpus.tables.stem_of(&word.form);
                let stop = self.corpus.tables.is_stop(&stem);
                let matches = self.matches(&word.term, &stem, typing && !required, required);
                query.asked.push(Asked { stop, matches });
                query.asked.len() - 1
            });
            if required && !query.required.contains(&i) {
                query.required.push(i);
            }
            query.order.push(i);
        };
        for (i, word) in free.into_iter().enumerate() {
            add(word, open && phrases.is_empty() && i == last, false, &mut query);
        }
        for phrase in &phrases {
            for word in words(phrase) {
                add(word, false, true, &mut query);
            }
        }
        query.asked.truncate(64);
        query.order.retain(|&i| i < 64);
        query
    }

    fn filter(&self, key: &str, value: &str) -> Option<Filter> {
        let value = plain(value);
        if value.is_empty() {
            return None;
        }
        if key.eq_ignore_ascii_case("in") {
            return Some(Filter::Names(self.names_like(&value)));
        }
        let field = self.corpus.fields.iter().position(|f| f.name.eq_ignore_ascii_case(key))?;
        Some(Filter::Field(field, value.split(' ').map(String::from).collect()))
    }

    /// The names a few typed words stand for: the one they name exactly, or every name holding a word each of
    /// them begins.
    fn names_like(&self, typed: &str) -> HashSet<u32> {
        if let Some(&id) = self.name_ids.get(typed) {
            return HashSet::from([id]);
        }
        let typed: Vec<&str> = typed.split(' ').collect();
        self.name_ids
            .iter()
            .filter(|(name, _)| {
                let have: Vec<&str> = name.split(' ').collect();
                typed.iter().all(|t| have.iter().any(|h| h.starts_with(t)))
            })
            .map(|(_, &id)| id)
            .collect()
    }

    /// A ref in a query: a name or alias, then numbers, perhaps ending in a range ("ham 3.1.56-60"). A name alone
    /// counts only when its records are keys.
    fn reference_in(&self, text: &str) -> Option<(u32, Vec<u32>, Option<u32>)> {
        let text = text.trim();
        let (name, last) = text.rsplit_once(' ').unwrap_or(("", text));
        let (from, until) = match last.split_once(['-', '\u{2013}']) {
            Some((from, until)) => (from, Some(until.parse::<u32>().ok()?)),
            None => (last, None),
        };
        match numbers(from) {
            Some(nums) if !name.trim().is_empty() => {
                let id = *self.name_ids.get(&plain(name))?;
                Some((id, nums, until))
            }
            _ => {
                let id = *self.name_ids.get(&plain(text))?;
                let first = *self.name_docs[id as usize].first()?;
                self.docs[first as usize].numbers.is_empty().then_some((id, Vec::new(), None))
            }
        }
    }

    /// The terms that stand for a query word: the word itself and its other forms. A word still being typed, or
    /// one that is in no record, also stands for the words it begins. A word with none of those is taken as a
    /// typo, and the terms an edit or two away stand for it. The corpus's synonyms come last, at less.
    fn matches(&self, word: &str, stem: &str, typing: bool, exact: bool) -> Vec<(u32, f32)> {
        let mut out: HashMap<u32, f32> = HashMap::new();
        let mut put = |term: u32, weight: f32| {
            let had = out.entry(term).or_insert(0.0);
            *had = had.max(weight);
        };

        let at = self.terms.partition_point(|term| term.as_str() < word);
        let mut near = false;
        if self.terms.get(at).is_some_and(|term| term == word) {
            put(at as u32, 1.0);
            near = true;
        }
        if let Some(group) = self.stems.get(stem) {
            for &term in group {
                put(term, STEM_WEIGHT);
                near = true;
            }
        }
        if exact {
            return out.into_iter().collect();
        }
        let length = word.chars().count();
        if typing || !near {
            let begun = self.terms[at..].iter().take_while(|term| term.starts_with(word)).count();
            let mut completions: Vec<u32> = (at..at + begun).map(|i| i as u32).collect();
            // A letter or two begins thousands of words. Past a budget of postings, only the commoner ones count.
            // A word that only adds, like a county in a place's region, is left to be typed out in full.
            let size = |t: &u32| self.postings[*t as usize].len();
            let mut floor = 1;
            while completions.iter().map(size).filter(|&n| n >= floor).sum::<usize>() > COMPLETION_BUDGET {
                floor = (floor * 2).max(2);
            }
            completions.retain(|t| size(t) >= floor);
            for term in completions {
                // The more of the term has been typed, the better the match.
                put(term, 0.3 + 0.5 * length as f32 / self.terms[term as usize].chars().count() as f32);
                near = true;
            }
        }
        if !near {
            let allowed = match length {
                0..=3 => 0,
                4..=7 => 1,
                _ => 2,
            };
            let typed: Vec<char> = word.chars().collect();
            let typed_letters = letter_set(word);
            for (i, term) in self.terms.iter().enumerate() {
                if allowed == 0 || term.len().abs_diff(word.len()) > allowed + 2 {
                    continue;
                }
                // A letter one word has and the other lacks takes an edit of its own.
                let (have, want) = (self.letters[i], typed_letters);
                if (have & !want).count_ones() as usize > allowed || (want & !have).count_ones() as usize > allowed {
                    continue;
                }
                let found = if word.is_ascii() && term.is_ascii() {
                    edits(word.as_bytes(), term.as_bytes(), allowed)
                } else {
                    edits(&typed, &term.chars().collect::<Vec<_>>(), allowed)
                };
                if let Some(edits) = found {
                    put(i as u32, TYPO_WEIGHTS[edits - 1]);
                }
            }
        }
        for (other, weight) in self.corpus.tables.synonyms(stem) {
            for &term in self.stems.get(other).map_or(&[][..], |v| v.as_slice()) {
                put(term, SYNONYM_WEIGHT * weight);
            }
        }
        out.into_iter().collect()
    }

    fn term(&self, word: &str) -> Option<u32> {
        self.terms.binary_search_by(|term| term.as_str().cmp(word)).ok().map(|i| i as u32)
    }

    fn show(&self, d: usize, query: &Query, wanted: &Map<u32, (usize, f32)>) -> Shown {
        let doc = &self.docs[d];
        let record = &self.corpus.records[d];
        // The sum, over the query's words, of the best match for each among these terms.
        let worth = |terms: &mut dyn Iterator<Item = u32>| -> f32 {
            let mut got = vec![0f32; query.asked.len()];
            for term in terms {
                if let Some(&(i, weight)) = wanted.get(&term) {
                    got[i] = got[i].max(weight);
                }
            }
            got.iter().sum()
        };

        // The best place in the record: a heading that matches, or the short run of words holding the most of the query.
        let mut best = 0.0;
        let mut place: Option<(usize, Option<(usize, usize)>)> = None;
        for (i, section) in record.sections.iter().enumerate() {
            let value = 1.5 * worth(&mut words(&section.heading).into_iter().filter_map(|w| self.term(&w.term)));
            if value > best {
                best = value;
                place = Some((i, None));
            }
        }
        for (i, first) in doc.words.iter().enumerate() {
            if !wanted.contains_key(&first.term) {
                continue;
            }
            let mut last = i;
            for (j, word) in doc.words.iter().enumerate().skip(i).take(WINDOW) {
                if word.section != first.section {
                    break;
                }
                if wanted.contains_key(&word.term) {
                    last = j;
                }
            }
            // Between two runs holding the same words, the tighter one wins.
            let value = worth(&mut doc.words[i..=last].iter().map(|w| w.term)) - 0.001 * (last - i) as f32;
            if value > best {
                best = value;
                place = Some((first.section as usize, Some((i, last))));
            }
        }

        let mut shown = Shown { doc: d, heading: String::new(), anchor: String::new(), snippet: Vec::new(), find: String::new() };
        let Some((section_index, run)) = place else {
            // The match is in a field. Show the description, or failing that how the record opens.
            let description = self.field("description", d).unwrap_or("");
            if !description.is_empty() {
                let spans: Vec<Span> = words(description)
                    .iter()
                    .map(|w| (w.start, w.end, self.term(&w.term).is_some_and(|t| wanted.contains_key(&t))))
                    .collect();
                shown.snippet = cut(description, &spans, 0, 0);
            } else if !record.sections.is_empty() {
                let (text, spans, _) = self.section(d, 0, &wanted);
                shown.snippet = cut(text, &spans, 0, 0);
            }
            return shown;
        };

        let section = &record.sections[section_index];
        shown.heading = section.heading.clone();
        shown.anchor = section.anchor.clone();
        let (text, spans, first_word) = self.section(d, section_index, &wanted);
        match run {
            Some((from, to)) => {
                let (from, to) = (from - first_word, to - first_word);
                shown.snippet = cut(text, &spans, from, to);
                let until = (from + 3).min(spans.len() - 1);
                shown.find = text[spans[from].0..spans[until].1].to_string();
            }
            None => shown.snippet = cut(text, &spans, 0, 0),
        }
        shown
    }

    /// A record's value for a named field.
    pub fn field(&self, name: &str, doc: usize) -> Option<&str> {
        let at = self.corpus.fields.iter().position(|f| f.name == name)?;
        self.corpus.records[doc].fields.get(at).map(String::as_str)
    }

    /// A section's text, its words as spans of that text, and where its words begin in the record's.
    fn section(&self, d: usize, index: usize, wanted: &Map<u32, (usize, f32)>) -> (&str, Vec<Span>, usize) {
        let doc = &self.docs[d];
        let first = doc.starts[index] as usize;
        let end = doc.starts.get(index + 1).map_or(doc.words.len(), |&next| next as usize);
        let spans = doc.words[first..end]
            .iter()
            .map(|w| (w.start as usize, w.end as usize, wanted.contains_key(&w.term)))
            .collect();
        (&self.corpus.records[d].sections[index].text, spans, first)
    }

    /// Every record's sections, for a caller that wants to read them.
    pub fn sections(&self, doc: usize) -> &[Section] {
        &self.corpus.records[doc].sections
    }

    pub fn names(&self) -> &[String] {
        &self.names
    }

    /// The record a ref names, the first when several share it.
    pub fn find(&self, reference: &str) -> Option<usize> {
        let (name, _) = split_ref(reference);
        let id = *self.name_ids.get(&plain(name))?;
        self.name_docs[id as usize].iter().map(|&d| d as usize).find(|&d| self.reference(d) == reference)
    }

    /// The records around one, under the same name: up to `before` ahead of it and `after` past it.
    pub fn around(&self, doc: usize, before: usize, after: usize) -> std::ops::Range<usize> {
        let name = self.docs[doc].name;
        let mut first = doc;
        while first > 0 && doc - first < before && self.docs[first - 1].name == name {
            first -= 1;
        }
        let mut last = doc;
        while last + 1 < self.docs.len() && last - doc < after && self.docs[last + 1].name == name {
            last += 1;
        }
        first..last + 1
    }
}

/// The letters a to z a word holds, a bit each, with one more bit for anything else.
fn letter_set(word: &str) -> u32 {
    word.bytes().fold(0, |set, b| set | if b.is_ascii_lowercase() { 1 << (b - b'a') } else { 1 << 26 })
}

/// Trim a text and turn every run of white space in it into one space.
fn squeeze(text: &mut String) {
    let mut clean = !text.starts_with(char::is_whitespace) && !text.ends_with(char::is_whitespace);
    let mut space = false;
    for c in text.chars() {
        if !clean {
            break;
        }
        let white = c == ' ' || (c > '\u{7f}' || c.is_ascii_whitespace()) && c.is_whitespace();
        clean = !(white && (c != ' ' || space));
        space = white;
    }
    if !clean {
        *text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    }
}

/// term -> (which word of the query it stands for, how good a match it is), for marking matched words.
fn marks(query: &Query) -> Map<u32, (usize, f32)> {
    let mut wanted: Map<u32, (usize, f32)> = Map::default();
    for (i, asked) in query.asked.iter().enumerate() {
        for &(term, weight) in &asked.matches {
            let had = wanted.entry(term).or_insert((i, weight));
            if weight > had.1 {
                *had = (i, weight);
            }
        }
    }
    wanted
}

/// For each term the query matched, which of its words the term stands for, as bits.
fn wanted_terms(query: &Query) -> Map<u32, u64> {
    let mut wanted: Map<u32, u64> = Map::default();
    for (i, asked) in query.asked.iter().enumerate() {
        for &(term, _) in &asked.matches {
            *wanted.entry(term).or_default() |= 1 << i;
        }
    }
    wanted
}

/// The query's pairs of neighbouring words, each pair once, weighted by the rarer word's rarity.
fn pairs(order: &[usize], rarity: &[f32]) -> Vec<(usize, usize, f32)> {
    let mut out: Vec<(usize, usize, f32)> = Vec::new();
    for w in order.windows(2) {
        if !out.iter().any(|p| (p.0, p.1) == (w[0], w[1])) {
            out.push((w[0], w[1], rarity[w[0]].min(rarity[w[1]])));
        }
    }
    out
}

/// A word of a text: its first byte, the byte after its last, and whether it matched the query.
type Span = (usize, usize, bool);

/// The stretch of `text` around words `from..=to`, widened to about SNIPPET_BYTES, as marked and unmarked pieces.
/// An ellipsis stands where text was left off.
fn cut(text: &str, spans: &[Span], from: usize, to: usize) -> Vec<(String, bool)> {
    if spans.is_empty() {
        return if text.is_empty() { Vec::new() } else { vec![(text.to_string(), false)] };
    }
    let (mut lo, mut hi) = (from, to);
    let fits = |lo: usize, hi: usize| spans[hi].1 - spans[lo].0 <= SNIPPET_BYTES;
    // A few words of lead-in, then as much after as fits, then more before if there is still room.
    for _ in 0..3 {
        if lo > 0 && fits(lo - 1, hi) {
            lo -= 1;
        }
    }
    while hi + 1 < spans.len() && fits(lo, hi + 1) {
        hi += 1;
    }
    while lo > 0 && fits(lo - 1, hi) {
        lo -= 1;
    }

    // Take the punctuation that touches the first and last words too.
    let begin = text[..spans[lo].0].rfind(' ').map_or(0, |space| space + 1);
    let end = text[spans[hi].1..].find(' ').map_or(text.len(), |space| spans[hi].1 + space);

    let mut pieces: Vec<(String, bool)> = Vec::new();
    let mut push = |piece: &str, marked: bool| {
        if piece.is_empty() {
            return;
        }
        match pieces.last_mut() {
            Some(last) if last.1 == marked => last.0.push_str(piece),
            _ => pieces.push((piece.to_string(), marked)),
        }
    };
    if lo > 0 {
        push("… ", false);
    }
    let mut at = begin;
    for &(start, stop, marked) in &spans[lo..=hi] {
        push(&text[at..start], false);
        push(&text[start..stop], marked);
        at = stop;
    }
    push(&text[at..end], false);
    if hi + 1 < spans.len() {
        push(" …", false);
    }
    pieces
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pages::{index_pages, Page};
    use crate::records::Section;

    fn section(anchor: &str, heading: &str, text: &str) -> Section {
        Section { anchor: anchor.into(), heading: heading.into(), text: text.into() }
    }

    fn page(url: &str, title: &str, description: &str, sections: Vec<Section>) -> Page {
        Page { url: url.into(), title: title.into(), description: description.into(), sections }
    }

    fn site() -> Index {
        index_pages(vec![
            page(
                "/recipes/panang-curry/",
                "Panang Curry",
                "A Thai curry for a weeknight.",
                vec![
                    section("", "", "This curry comes together in half an hour."),
                    section("ingredients", "Ingredients", "One can of coconut milk, two spoons of peanut butter, and a lime."),
                    section("directions", "Directions", "Fry the paste. Don't let the coconut milk boil."),
                ],
            ),
            page("/recipes/oat-balls/", "Oat and peanut butter balls", "", vec![section("", "", "Roll the oats, the honey and the peanut butter into balls. Chill them.")]),
            page("/parks/", "National parks", "All 63 national parks in shaded relief.", Vec::new()),
            page("/parks/mount-rainier/", "Mount Rainier", "Mount Rainier in shaded relief.", Vec::new()),
            page("/essays/reduce-friction/", "Reduce friction", "", vec![section("", "", "A park bench by the door is the whole trick: make the next step small.")]),
        ])
    }

    fn urls(index: &Index, query: &str) -> Vec<String> {
        index.search(query, 10).into_iter().map(|h| index.reference(h.shown[0].doc).to_string()).collect()
    }

    #[test]
    fn empty_query_returns_nothing() {
        assert!(site().search("", 10).is_empty());
        assert!(site().search(" ,. ", 10).is_empty());
    }

    #[test]
    fn every_word_of_every_page_finds_its_page() {
        let index = site();
        for (d, record) in index.corpus.records.iter().enumerate() {
            let mut text = record.fields.join(" ");
            for s in &record.sections {
                text.push_str(&format!(" {} {}", s.heading, s.text));
            }
            for word in words(&text) {
                assert!(urls(&index, &word.term).contains(&index.reference(d).to_string()), "{:?} did not find {}", word.term, record.reference);
            }
        }
    }

    #[test]
    fn a_page_with_all_the_words_beats_a_page_with_some() {
        let found = urls(&site(), "peanut butter balls");
        assert_eq!(found[0], "/recipes/oat-balls/");
        assert!(found.contains(&"/recipes/panang-curry/".to_string()));
    }

    #[test]
    fn a_boost_orders_equal_matches_and_never_beats_more_words() {
        let records = "#ref\ttext\tboost\n\
            Portland\tPortland\t0\n\
            Portland\tPortland\t0.4\n\
            Portland Head Light\tPortland Head Light\t2\n\
            Head Island\tHead Island\t0\n";
        let index = Index::new(Corpus::from_tsv(records).unwrap());
        let docs = |q: &str| index.search(q, 10).iter().map(|h| h.shown.iter().map(|s| s.doc).collect::<Vec<_>>()).collect::<Vec<_>>();
        assert_eq!(docs("portland ")[..2], [vec![1, 0], vec![2]]);
        assert_eq!(docs("head ")[..2], [vec![2], vec![3]]);
        assert_eq!(docs("head island ")[0], [3]);
    }

    #[test]
    fn a_region_narrows_a_name_and_finds_nothing_alone() {
        let records = "#ref\ttext\talt*1\tregion+1\tboost\n\
            Portland\tPortland\tPortland Maine\tMaine United States\t1.2\n\
            Portland\tPortland\tRose City\tOregon United States\t1.0\n\
            Portland\tPortland\t\tVictoria Australia\t0.5\n\
            Springfield\tSpringfield\t\tIllinois United States\t1.1\n\
            Springfield\tSpringfield\t\tMissouri United States\t0.9\n\
            Paris\tParis\tLutetia\tFrance\t1.5\n\
            Paris\tParis\t\tTexas United States\t0.2\n\
            Oregon Zoo\tOregon Zoo\t\tPortland United States\t0.3\n\
            Mount Hood\tMount Hood\t\tClackamas Oregon United States\t0.6\n\
            Texas State Capitol\tTexas State Capitol\t\tAustin United States\t0.4\n";
        let index = Index::new(Corpus::from_tsv(records).unwrap());
        let docs = |q: &str| index.search(q, 10).iter().flat_map(|h| h.shown.iter().map(|s| s.doc)).collect::<Vec<_>>();
        // Portland in Oregon and the zoo in Portland both hold both words, so they come before Portland in Maine.
        let mut both = docs("portland oregon ")[..3].to_vec();
        assert_eq!(both.pop(), Some(0));
        both.sort();
        assert_eq!(both, [1, 7]);
        assert_eq!(docs("portland ore")[2], 0);
        assert_eq!(docs("portland maine ")[0], 0);
        assert_eq!(docs("springfield missouri ")[0], 4);
        assert_eq!(docs("springfield illinois ")[0], 3);
        assert_eq!(docs("paris texas ")[0], 6);
        assert_eq!(docs("paris france ")[0], 5);
        assert_eq!(docs("hood oregon ")[0], 8);
        // A plain name ranks by boost, and a region word alone finds only what its name or text holds.
        assert_eq!(docs("portland "), [0, 1, 2]);
        assert_eq!(docs("paris "), [5, 6]);
        assert_eq!(docs("oregon "), [7]);
        assert_eq!(docs("texas "), [9]);
        assert!(docs("clackamas ").is_empty());
    }

    #[test]
    fn a_title_match_beats_a_body_match() {
        assert_eq!(urls(&site(), "curry")[0], "/recipes/panang-curry/");
        assert_eq!(urls(&site(), "parks")[0], "/parks/");
    }

    #[test]
    fn an_exact_word_still_finds_the_words_it_begins() {
        // "park" is a word on the essay's page, and it also begins "parks".
        let found = urls(&site(), "park");
        assert!(found.contains(&"/essays/reduce-friction/".to_string()));
        assert!(found.contains(&"/parks/".to_string()));
    }

    #[test]
    fn only_the_last_word_is_taken_as_unfinished() {
        // "a" begins "all" and "and", but as a finished word it is only itself.
        let marked = |query: &str| -> Vec<String> {
            let index = site();
            let hit = index.search(query, 10).into_iter().find(|h| index.reference(h.shown[0].doc) == "/recipes/panang-curry/").unwrap();
            hit.shown[0].snippet.iter().filter(|p| p.1).map(|p| p.0.clone()).collect()
        };
        assert_eq!(marked("a lime"), ["a", "lime"]);
        assert_eq!(marked("lime a"), ["and", "a", "lime"]);
    }

    #[test]
    fn other_forms_of_a_word_match() {
        assert_eq!(urls(&site(), "currys")[0], "/recipes/panang-curry/");
        assert_eq!(urls(&site(), "frictions")[0], "/essays/reduce-friction/");
        assert_eq!(urls(&site(), "reducing")[0], "/essays/reduce-friction/");
    }

    #[test]
    fn a_typo_still_finds_the_page() {
        assert_eq!(urls(&site(), "raineir")[0], "/parks/mount-rainier/");
        assert_eq!(urls(&site(), "cocnut")[0], "/recipes/panang-curry/");
        assert_eq!(urls(&site(), "ingrediants")[0], "/recipes/panang-curry/");
    }

    #[test]
    fn one_letter_is_a_search() {
        assert!(urls(&site(), "r").contains(&"/parks/mount-rainier/".to_string()));
    }

    #[test]
    fn an_apostrophe_is_optional() {
        assert_eq!(urls(&site(), "dont boil")[0], "/recipes/panang-curry/");
    }

    #[test]
    fn nonsense_finds_nothing() {
        assert!(site().search("xyzzyq", 10).is_empty());
    }

    #[test]
    fn the_result_points_at_the_section_and_marks_the_words() {
        let index = site();
        let shown = &index.search("peanut lime", 10)[0].shown[0];
        assert_eq!(index.reference(shown.doc), "/recipes/panang-curry/");
        assert_eq!(shown.anchor, "ingredients");
        assert_eq!(shown.heading, "Ingredients");
        let marked: Vec<&str> = shown.snippet.iter().filter(|p| p.1).map(|p| p.0.as_str()).collect();
        assert_eq!(marked, ["peanut", "lime"]);
        assert_eq!(shown.find, "peanut butter, and a");
    }

    #[test]
    fn a_heading_match_points_at_its_section() {
        let shown = &site().search("directions", 10)[0].shown[0];
        assert_eq!(shown.anchor, "directions");
        assert_eq!(shown.snippet[0].0, "Fry the paste. Don't let the coconut milk boil.");
    }

    #[test]
    fn a_match_only_in_the_title_shows_the_description() {
        let shown = &site().search("panang", 10)[0].shown[0];
        assert_eq!(shown.anchor, "");
        let text: String = shown.snippet.iter().map(|p| p.0.as_str()).collect();
        assert_eq!(text, "A Thai curry for a weeknight.");
    }

    #[test]
    fn a_long_text_is_cut_around_the_match() {
        let filler = "lorem ipsum dolor sit amet ".repeat(40);
        let index = index_pages(vec![page("/long/", "Long", "", vec![section("", "", &format!("{filler}the needle is here {filler}"))])]);
        let text: String = index.search("needle", 10)[0].shown[0].snippet.iter().map(|p| p.0.as_str()).collect();
        assert!(text.starts_with("… ") && text.ends_with(" …"));
        assert!(text.contains("the needle is here"));
        assert!(text.len() < SNIPPET_BYTES + 20);
    }

    const PLAY: &str = "#ref\ttext\tspeaker\tkind\n\
        Hamlet 3.1.55\tEnter Hamlet.\t\tstage\n\
        Hamlet 3.1.56\tTo be, or not to be, that is the question:\tHamlet\tverse\n\
        Hamlet 3.1.57\tWhether 'tis nobler in the mind to suffer\tHamlet\tverse\n\
        Hamlet 3.1.58\tThe slings and arrows of outrageous fortune,\tHamlet\tverse\n\
        Hamlet 3.2.1\tSpeak the speech, I pray you, as I pronounc'd it to you,\tHamlet\tprose\n\
        Macbeth 5.5.19\tTomorrow, and tomorrow, and tomorrow,\tMacbeth\tverse\n\
        Macbeth 5.5.20\tCreeps in this petty pace from day to day,\tMacbeth\tverse\n\
        Macbeth 5.5.21\tNot to be or to be is not the question here,\tMacbeth\tverse\n";

    fn play() -> Index {
        let mut corpus = Corpus::from_tsv(PLAY).unwrap();
        corpus.tables.add("aliases", "Hamlet\tHam\n").unwrap();
        corpus.tables.add("stop", "the\tto\tis\tin\tof\tand\n").unwrap();
        corpus.tables.add("suffixes", "'d\ted\n").unwrap();
        corpus.tables.add("synonyms", "throw\tsling\n").unwrap();
        Index::new(corpus)
    }

    fn refs(index: &Index, query: &str) -> Vec<String> {
        index.search(query, 10).into_iter().map(|h| index.span(h.shown[0].doc, h.shown.last().unwrap().doc)).collect()
    }

    #[test]
    fn words_in_order_beat_the_same_words_out_of_order() {
        assert_eq!(refs(&play(), "to be or not")[0], "Hamlet 3.1.56");
    }

    #[test]
    fn a_passage_across_lines_is_one_result() {
        assert_eq!(refs(&play(), "the question whether tis nobler")[0], "Hamlet 3.1.56-57");
    }

    #[test]
    fn a_ref_goes_straight_to_its_record() {
        assert_eq!(refs(&play(), "hamlet 3.1.57"), ["Hamlet 3.1.57"]);
        assert_eq!(refs(&play(), "ham 3.2"), ["Hamlet 3.2.1"]);
        assert_eq!(refs(&play(), "ham 3.1.56-58"), ["Hamlet 3.1.56-58"]);
    }

    #[test]
    fn filters_narrow_a_search() {
        assert_eq!(refs(&play(), "speaker:macbeth to be"), ["Macbeth 5.5.21"]);
        assert_eq!(refs(&play(), "in:mac question"), ["Macbeth 5.5.21"]);
        assert_eq!(refs(&play(), "kind:stage enter"), ["Hamlet 3.1.55"]);
    }

    #[test]
    fn stop_words_add_to_a_match_but_never_make_one() {
        assert!(refs(&play(), "of the").contains(&"Hamlet 3.1.58".to_string()));
        assert!(!refs(&play(), "tomorrow the").contains(&"Hamlet 3.1.56".to_string()));
    }

    #[test]
    fn tables_join_old_forms_and_synonyms() {
        assert_eq!(refs(&play(), "pronounced")[0], "Hamlet 3.2.1");
        assert_eq!(refs(&play(), "throws and arrows")[0], "Hamlet 3.1.58");
    }

    #[test]
    fn a_quoted_phrase_must_be_there() {
        assert_eq!(refs(&play(), "\"petty pace\""), ["Macbeth 5.5.20"]);
        assert!(refs(&play(), "\"pety\"").is_empty());
    }
}
