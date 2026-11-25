//! Text files into records. Markdown and HTML split at their headings, anything else at blank lines, unless told
//! to split by line, paragraph or heading. A record's ref is its file's path and a running number, its title the
//! document's title, and its heading and anchor the heading it sits under.

use crate::records::{Corpus, Field, Record, Section};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum By {
    Line,
    Paragraph,
    Heading,
}

impl By {
    pub fn parse(word: &str) -> Option<By> {
        match word {
            "line" => Some(By::Line),
            "paragraph" => Some(By::Paragraph),
            "heading" => Some(By::Heading),
            _ => None,
        }
    }
}

/// The fields every split record has.
pub fn fields() -> Vec<Field> {
    vec![Field { name: "title".into(), count: 1.0, bonus: 2.0, adds: false }]
}

/// Files, as (path, text), into a corpus.
pub fn corpus(files: &[(String, String)], by: Option<By>) -> Corpus {
    let mut corpus = Corpus { fields: fields(), ..Corpus::default() };
    for (path, text) in files {
        corpus.records.extend(split(path, text, by));
    }
    corpus
}

pub fn split(path: &str, text: &str, by: Option<By>) -> Vec<Record> {
    let lower = path.to_lowercase();
    let kind = lower.rsplit('.').next().unwrap_or("");
    let (title, sections) = match (kind, by) {
        ("md" | "markdown", None | Some(By::Heading)) => markdown(text),
        ("html" | "htm", None | Some(By::Heading)) => html(text),
        ("html" | "htm", Some(by)) => {
            let (title, sections) = html(text);
            let joined = sections.into_iter().map(|s| s.text).collect::<Vec<_>>().join("\n\n");
            (title, pieces(&joined, by))
        }
        (_, Some(By::Heading)) => markdown(text),
        (_, by) => (String::new(), pieces(text, by.unwrap_or(By::Paragraph))),
    };
    let title = if title.is_empty() { file_name(path).to_string() } else { title };
    sections
        .into_iter()
        .filter(|s| !s.text.trim().is_empty() || !s.heading.is_empty())
        .enumerate()
        .map(|(i, section)| Record { reference: format!("{path} {}", i + 1), boost: 0.0, fields: vec![title.clone()], sections: vec![section] })
        .collect()
}

fn file_name(path: &str) -> &str {
    path.rsplit(['/', '\\']).next().unwrap_or(path)
}

fn pieces(text: &str, by: By) -> Vec<Section> {
    let parts: Vec<String> = match by {
        By::Line => text.lines().map(String::from).collect(),
        _ => {
            let mut parts = vec![String::new()];
            for line in text.lines() {
                if line.trim().is_empty() {
                    if !parts.last().unwrap().is_empty() {
                        parts.push(String::new());
                    }
                } else {
                    let last = parts.last_mut().unwrap();
                    if !last.is_empty() {
                        last.push(' ');
                    }
                    last.push_str(line.trim());
                }
            }
            parts
        }
    };
    parts.into_iter().map(|text| Section { text, ..Section::default() }).collect()
}

/// A heading's id the way most site generators make it: lower case, words joined by hyphens.
pub fn slug(heading: &str) -> String {
    let mut out = String::new();
    for c in heading.chars().flat_map(char::to_lowercase) {
        if c.is_alphanumeric() {
            out.push(c);
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
    }
    out.trim_end_matches('-').to_string()
}

/// A Markdown file's title (from its front matter or first top heading) and its sections.
fn markdown(text: &str) -> (String, Vec<Section>) {
    let mut title = String::new();
    let mut body = text;
    for fence in ["+++", "---"] {
        if let Some(rest) = text.strip_prefix(fence) {
            if let Some((front, after)) = rest.split_once(&format!("\n{fence}")) {
                for line in front.lines() {
                    let line = line.trim();
                    if let Some(value) = line.strip_prefix("title").map(str::trim_start).and_then(|v| v.strip_prefix(['=', ':'])) {
                        title = value.trim().trim_matches(['"', '\'']).to_string();
                    }
                }
                body = after.split_once('\n').map_or("", |(_, b)| b);
            }
        }
    }
    let mut sections = vec![Section::default()];
    let mut fenced = false;
    for line in body.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            fenced = !fenced;
            continue;
        }
        let level = trimmed.bytes().take_while(|&b| b == b'#').count();
        if !fenced && (1..=6).contains(&level) && trimmed[level..].starts_with(' ') {
            let heading = inline(trimmed[level..].trim().trim_end_matches('#').trim());
            if level == 1 && title.is_empty() {
                title = heading.clone();
            }
            sections.push(Section { anchor: slug(&heading), heading, text: String::new() });
            continue;
        }
        let text = if fenced { line.to_string() } else { inline(strip_marker(trimmed)) };
        let last = &mut sections.last_mut().unwrap().text;
        if !last.is_empty() {
            last.push(if text.is_empty() { '\n' } else { ' ' });
        }
        last.push_str(&text);
    }
    for section in &mut sections {
        section.text = section.text.trim().to_string();
    }
    (title, sections)
}

/// A line without its quote or list marker.
fn strip_marker(line: &str) -> &str {
    let line = line.trim_start_matches('>').trim_start();
    for marker in ["- ", "* ", "+ "] {
        if let Some(rest) = line.strip_prefix(marker) {
            return rest;
        }
    }
    let digits = line.bytes().take_while(u8::is_ascii_digit).count();
    if digits > 0 && line[digits..].starts_with(". ") {
        return &line[digits + 2..];
    }
    line
}

/// Markdown's inline markup taken out: links and images keep their text, emphasis and code marks go.
fn inline(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(c) = rest.chars().next() {
        if c == '!' && rest[1..].starts_with('[') {
            rest = &rest[1..];
            continue;
        }
        if c == '[' {
            if let Some(close) = rest.find("](") {
                if let Some(end) = rest[close..].find(')') {
                    out.push_str(&rest[1..close]);
                    rest = &rest[close + end + 1..];
                    continue;
                }
            }
        }
        if c == '<' {
            if let Some(end) = rest.find('>') {
                rest = &rest[end + 1..];
                continue;
            }
        }
        if !matches!(c, '*' | '_' | '`') || out.ends_with(char::is_alphanumeric) && rest[1..].starts_with(char::is_alphanumeric) {
            out.push(c);
        }
        rest = &rest[c.len_utf8()..];
    }
    out
}

/// An HTML file's title and its sections, cut at headings that have an id.
fn html(source: &str) -> (String, Vec<Section>) {
    let mut title = String::new();
    let mut sections = vec![Section::default()];
    let mut rest = source;
    let mut in_heading: Option<(String, String)> = None;
    while !rest.is_empty() {
        let Some(open) = rest.find('<') else {
            push_text(&mut sections, &mut in_heading, rest);
            break;
        };
        push_text(&mut sections, &mut in_heading, &rest[..open]);
        rest = &rest[open..];
        if rest.starts_with("<!--") {
            rest = rest.find("-->").map_or("", |end| &rest[end + 3..]);
            continue;
        }
        let close = rest.find('>').map_or(rest.len(), |c| c + 1);
        let tag = &rest[..close];
        rest = &rest[close..];
        let name: String = tag.trim_start_matches(['<', '/']).chars().take_while(|c| c.is_ascii_alphanumeric()).collect::<String>().to_ascii_lowercase();
        let closing = tag.starts_with("</");
        if !closing && matches!(name.as_str(), "script" | "style" | "template" | "title") {
            let end = format!("</{name}");
            let at = rest.to_ascii_lowercase().find(&end).unwrap_or(rest.len());
            if name == "title" {
                title = entities(&rest[..at]).split_whitespace().collect::<Vec<_>>().join(" ");
            }
            rest = &rest[at..];
            continue;
        }
        let is_heading = name.len() == 2 && name.starts_with('h') && name.as_bytes()[1].is_ascii_digit();
        if is_heading && !closing {
            if let Some(id) = attribute(tag, "id") {
                in_heading = Some((id, String::new()));
                continue;
            }
        }
        if is_heading && closing {
            if let Some((anchor, heading)) = in_heading.take() {
                let heading = heading.split_whitespace().collect::<Vec<_>>().join(" ");
                sections.push(Section { anchor, heading, text: String::new() });
                continue;
            }
        }
        if !matches!(name.as_str(), "a" | "abbr" | "b" | "cite" | "code" | "em" | "i" | "kbd" | "mark" | "q" | "s" | "small" | "span" | "strong" | "sub" | "sup" | "u") {
            push_text(&mut sections, &mut in_heading, " ");
        }
    }
    (title, sections)
}

fn push_text(sections: &mut [Section], in_heading: &mut Option<(String, String)>, text: &str) {
    let text = entities(text);
    match in_heading {
        Some((_, heading)) => heading.push_str(&text),
        None => sections.last_mut().unwrap().text.push_str(&text),
    }
}

fn attribute(tag: &str, name: &str) -> Option<String> {
    let lower = tag.to_ascii_lowercase();
    let at = lower.find(&format!(" {name}="))? + name.len() + 2;
    let value = &tag[at..];
    let value = match value.chars().next()? {
        q @ ('"' | '\'') => value[1..].split(q).next()?,
        _ => value.split([' ', '>', '/']).next()?,
    };
    Some(entities(value))
}

fn entities(text: &str) -> String {
    if !text.contains('&') {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        rest = &rest[at..];
        let Some(end) = rest[..rest.len().min(12)].find(';') else {
            out.push('&');
            rest = &rest[1..];
            continue;
        };
        let name = &rest[1..end];
        let decoded = match name {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            "nbsp" => Some(' '),
            "mdash" => Some('—'),
            "ndash" => Some('–'),
            "rsquo" => Some('’'),
            "lsquo" => Some('‘'),
            "rdquo" => Some('”'),
            "ldquo" => Some('“'),
            "hellip" => Some('…'),
            _ => name
                .strip_prefix("#x")
                .or_else(|| name.strip_prefix("#X"))
                .and_then(|h| u32::from_str_radix(h, 16).ok())
                .or_else(|| name.strip_prefix('#').and_then(|d| d.parse().ok()))
                .and_then(char::from_u32),
        };
        match decoded {
            Some(c) => {
                out.push(c);
                rest = &rest[end + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markdown_splits_at_headings() {
        let text = "+++\ntitle = \"Panang curry\"\n+++\nA Thai *curry*.\n\n## Ingredients\n\n- One can of [coconut milk](/milk/)\n- A lime\n\n```\nfry_paste()\n```\n";
        let records = split("recipes/panang.md", text, None);
        assert_eq!(records.len(), 2);
        assert_eq!(records[0].reference, "recipes/panang.md 1");
        assert_eq!(records[0].fields, ["Panang curry"]);
        assert_eq!(records[0].sections[0].text, "A Thai curry.");
        let ingredients = &records[1].sections[0];
        assert_eq!((ingredients.heading.as_str(), ingredients.anchor.as_str()), ("Ingredients", "ingredients"));
        assert!(ingredients.text.contains("One can of coconut milk"));
        assert!(ingredients.text.contains("fry_paste()"));
    }

    #[test]
    fn a_markdown_title_falls_back_to_the_first_heading_then_the_file() {
        assert_eq!(split("a.md", "# How search works\n\nText.", None)[0].fields, ["How search works"]);
        assert_eq!(split("notes/a.md", "Just text.", None)[0].fields, ["a.md"]);
    }

    #[test]
    fn html_splits_at_headings_with_ids() {
        let text = "<html><head><title>Tide &amp; time</title><script>var x = '<h2 id=no>';</script></head><body>\
            <p>Before the <b>first</b> heading.</p><h2 id=\"high\">High <em>tide</em></h2><p>Water&nbsp;rises.</p></body></html>";
        let records = split("tide.html", text, None);
        assert_eq!(records[0].fields, ["Tide & time"]);
        assert_eq!(records[0].sections[0].text.split_whitespace().collect::<Vec<_>>().join(" "), "Before the first heading.");
        assert_eq!(records[1].sections[0].anchor, "high");
        assert_eq!(records[1].sections[0].heading, "High tide");
        assert!(records[1].sections[0].text.contains("Water rises."));
    }

    #[test]
    fn plain_text_splits_at_blank_lines_or_lines() {
        let text = "One line\nand its next.\n\n\nA second paragraph.\n";
        let paragraphs = split("notes.txt", text, None);
        assert_eq!(paragraphs.iter().map(|r| r.sections[0].text.as_str()).collect::<Vec<_>>(), ["One line and its next.", "A second paragraph."]);
        assert_eq!(paragraphs[1].reference, "notes.txt 2");
        assert_eq!(split("notes.txt", text, Some(By::Line)).len(), 3);
    }
}
