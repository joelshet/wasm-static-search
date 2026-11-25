//! A website's pages, as `load_pages` takes them: each page's address, title, description, and its text cut into
//! sections at its headings. A page is one record: a title word counts once and adds twice its rarity, a
//! description word and a heading word count twice.

use crate::index::Index;
use crate::json::{self, Json};
use crate::records::{Corpus, Field, Record, Section};

#[derive(Clone, Debug, Default)]
pub struct Page {
    pub url: String,
    pub title: String,
    pub description: String,
    pub sections: Vec<Section>,
}

/// Pages from JSON: `[{url, title, description, sections: [{anchor, heading, text}]}]`. None when the JSON is not
/// that.
pub fn pages_from_json(source: &str) -> Option<Vec<Page>> {
    let value = json::parse(source).ok()?;
    let mut pages = Vec::new();
    for page in value.list()? {
        let text = |value: &Json, key: &str| value.get(key).map_or(Some(""), Json::text).map(String::from);
        let mut sections = Vec::new();
        if let Some(list) = page.get("sections") {
            for section in list.list()? {
                sections.push(Section { anchor: text(section, "anchor")?, heading: text(section, "heading")?, text: text(section, "text")? });
            }
        }
        pages.push(Page { url: page.get("url")?.text()?.to_string(), title: page.get("title")?.text()?.to_string(), description: text(page, "description")?, sections });
    }
    Some(pages)
}

pub fn index_pages(pages: Vec<Page>) -> Index {
    let fields = vec![Field { name: "title".into(), count: 1.0, bonus: 2.0, adds: false }, Field { name: "description".into(), count: 2.0, bonus: 0.0, adds: false }];
    let records = pages
        .into_iter()
        .map(|page| Record { reference: page.url, boost: 0.0, fields: vec![page.title, page.description], sections: page.sections })
        .collect();
    Index::new(Corpus { fields, records, ..Corpus::default() })
}
