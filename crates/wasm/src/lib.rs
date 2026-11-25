use std::cell::RefCell;
use wasm_bindgen::prelude::*;

use search_core::index::{Hit, Index, Shown};
use search_core::json::quote;
use search_core::pages::{index_pages, pages_from_json};

thread_local! {
    static INDEX: RefCell<Option<Index>> = const { RefCell::new(None) };
}

/// Index a site's pages, given as a JSON array of {url, title, description, sections: [{anchor, heading, text}]}.
/// Returns false, and keeps any index already loaded, when the JSON is not that.
#[wasm_bindgen]
pub fn load_pages(json: &str) -> bool {
    let Some(pages) = pages_from_json(json) else { return false };
    INDEX.with(|cell| cell.replace(Some(index_pages(pages))));
    true
}

/// Search the loaded index. Returns a JSON array of results. Each is {url, title, heading, anchor, snippet, find,
/// score} for its first record, with `ref` (the whole result's ref, "Hamlet 3.1.56-57" for a passage), `fields`
/// (the record's fields by name), and `more` (the passage's other records, each {url, heading, anchor, snippet,
/// fields}). `snippet` is the text around the match as [text, matched] pieces.
#[wasm_bindgen]
pub fn search(query: &str, max: usize) -> String {
    INDEX.with(|cell| match cell.borrow().as_ref() {
        Some(index) => results_to_json(index, &index.search(query, max)),
        None => "[]".into(),
    })
}

/// Load an index built by `search-cli index`. Returns false, and keeps any index already loaded, when the bytes
/// are not one.
#[cfg(feature = "files")]
#[wasm_bindgen]
pub fn load_index(bytes: &[u8]) -> bool {
    match search_core::records::Corpus::from_bytes(bytes) {
        Ok(corpus) => {
            INDEX.with(|cell| cell.replace(Some(Index::new(corpus))));
            true
        }
        Err(_) => false,
    }
}

/// Index files dropped on a page, given as a JSON array of {path, text}: split, then indexed, here in the browser.
/// `by` is "", "line", "paragraph" or "heading". Returns how many records came out, or -1 for bad JSON.
#[cfg(feature = "files")]
#[wasm_bindgen]
pub fn index_files(json: &str, by: &str) -> i32 {
    use search_core::json::{parse, Json};
    use search_core::records::Corpus;
    let Ok(value) = parse(json) else { return -1 };
    let Some(list) = value.list() else { return -1 };
    let mut files = Vec::new();
    let mut corpus = Corpus::default();
    for item in list {
        let (Some(path), Some(text)) = (item.get("path").and_then(Json::text), item.get("text").and_then(Json::text)) else { return -1 };
        let name = path.rsplit('/').next().unwrap_or(path);
        let table = name.strip_suffix(".tsv").filter(|t| search_core::tables::NAMES.contains(t));
        match table {
            Some(table) if corpus.tables.add(table, text).is_ok() => {}
            _ if name.ends_with(".tsv") && text.starts_with("#ref") => match Corpus::from_tsv(text) {
                Ok(records) if corpus.records.is_empty() || corpus.fields == records.fields => {
                    corpus.fields = records.fields;
                    corpus.records.extend(records.records);
                }
                _ => {}
            },
            _ => files.push((path.to_string(), text.to_string())),
        }
    }
    let split = search_core::split::corpus(&files, search_core::split::By::parse(by));
    if corpus.records.is_empty() {
        corpus.fields = split.fields;
        corpus.records = split.records;
    } else if !split.records.is_empty() && corpus.fields == split.fields {
        corpus.records.extend(split.records);
    }
    let count = corpus.records.len() as i32;
    INDEX.with(|cell| cell.replace(Some(Index::new(corpus))));
    count
}

/// What the loaded index holds: {records, fields: [names], names: [ref names]}.
#[cfg(feature = "files")]
#[wasm_bindgen]
pub fn meta() -> String {
    INDEX.with(|cell| {
        let index = cell.borrow();
        let Some(index) = index.as_ref() else { return "null".into() };
        let mut out = format!("{{\"records\":{},\"fields\":[", index.len());
        list(&mut out, index.corpus.fields.iter().map(|f| f.name.as_str()));
        out.push_str("],\"names\":[");
        list(&mut out, index.names().iter().map(String::as_str));
        out.push_str("]}");
        out
    })
}

/// The records around a ref, under the same name, as a JSON array of {url, text, fields}, with the ref's own
/// record marked `"here": true`.
#[cfg(feature = "files")]
#[wasm_bindgen]
pub fn around(reference: &str, before: usize, after: usize) -> String {
    INDEX.with(|cell| {
        let index = cell.borrow();
        let Some(index) = index.as_ref() else { return "[]".into() };
        let Some(doc) = index.find(reference) else { return "[]".into() };
        let mut out = String::from("[");
        for (i, d) in index.around(doc, before, after).enumerate() {
            if i > 0 {
                out.push(',');
            }
            out.push_str("{\"url\":");
            quote(index.reference(d), &mut out);
            out.push_str(",\"text\":");
            let text: Vec<&str> = index.sections(d).iter().map(|s| s.text.as_str()).collect();
            quote(&text.join(" "), &mut out);
            out.push_str(",\"fields\":{");
            for (k, field) in index.corpus.fields.iter().enumerate() {
                if k > 0 {
                    out.push(',');
                }
                quote(&field.name, &mut out);
                out.push(':');
                quote(&index.corpus.records[d].fields[k], &mut out);
            }
            out.push('}');
            if d == doc {
                out.push_str(",\"here\":true");
            }
            out.push('}');
        }
        out.push(']');
        out
    })
}

#[cfg(feature = "files")]
fn list<'a>(out: &mut String, items: impl Iterator<Item = &'a str>) {
    for (i, item) in items.enumerate() {
        if i > 0 {
            out.push(',');
        }
        quote(item, out);
    }
}

fn results_to_json(index: &Index, hits: &[Hit]) -> String {
    let mut out = String::from("[");
    for (i, hit) in hits.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        let first = &hit.shown[0];
        out.push('{');
        record_json(index, first, &mut out);
        out.push_str(",\"title\":");
        quote(index.field("title", first.doc).unwrap_or(""), &mut out);
        out.push_str(",\"find\":");
        quote(&first.find, &mut out);
        // To three places, by integer arithmetic: formatting a float would add a fifth to the wasm.
        let thousandths = if hit.score.is_finite() { (hit.score.max(0.0) * 1000.0).round() as u64 } else { 0 };
        out.push_str(&format!(",\"score\":{}.{:03}", thousandths / 1000, thousandths % 1000));
        out.push_str(",\"ref\":");
        quote(&index.span(first.doc, hit.shown.last().unwrap().doc), &mut out);
        out.push_str(",\"more\":[");
        for (j, shown) in hit.shown[1..].iter().enumerate() {
            if j > 0 {
                out.push(',');
            }
            out.push('{');
            record_json(index, shown, &mut out);
            out.push('}');
        }
        out.push_str("]}");
    }
    out.push(']');
    out
}

fn record_json(index: &Index, shown: &Shown, out: &mut String) {
    out.push_str("\"url\":");
    quote(index.reference(shown.doc), out);
    out.push_str(",\"heading\":");
    quote(&shown.heading, out);
    out.push_str(",\"anchor\":");
    quote(&shown.anchor, out);
    out.push_str(",\"snippet\":[");
    for (k, (text, marked)) in shown.snippet.iter().enumerate() {
        if k > 0 {
            out.push(',');
        }
        out.push('[');
        quote(text, out);
        out.push_str(if *marked { ",true]" } else { ",false]" });
    }
    out.push_str("],\"fields\":{");
    for (k, field) in index.corpus.fields.iter().enumerate() {
        if k > 0 {
            out.push(',');
        }
        quote(&field.name, out);
        out.push(':');
        quote(index.corpus.records[shown.doc].fields.get(k).map_or("", String::as_str), out);
    }
    out.push('}');
}
