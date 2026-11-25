//! Just enough JSON to read the pages a site hands in and to write results back.

#[derive(Debug, Clone, PartialEq)]
pub enum Json {
    Null,
    Bool(bool),
    Number(f64),
    Text(String),
    List(Vec<Json>),
    Object(Vec<(String, Json)>),
}

impl Json {
    pub fn get(&self, key: &str) -> Option<&Json> {
        match self {
            Json::Object(pairs) => pairs.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    /// A string's text; null and a missing value read as empty.
    pub fn text(&self) -> Option<&str> {
        match self {
            Json::Text(s) => Some(s),
            Json::Null => Some(""),
            _ => None,
        }
    }

    pub fn list(&self) -> Option<&[Json]> {
        match self {
            Json::List(items) => Some(items),
            _ => None,
        }
    }
}

pub fn parse(source: &str) -> Result<Json, String> {
    let mut reader = Reader { bytes: source.as_bytes(), at: 0 };
    let value = reader.value()?;
    reader.space();
    if reader.at != reader.bytes.len() {
        return Err(format!("JSON: unexpected text at byte {}", reader.at));
    }
    Ok(value)
}

struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl Reader<'_> {
    fn space(&mut self) {
        while self.bytes.get(self.at).is_some_and(|b| b.is_ascii_whitespace()) {
            self.at += 1;
        }
    }

    fn fail<T>(&self, what: &str) -> Result<T, String> {
        Err(format!("JSON: {what} at byte {}", self.at))
    }

    fn eat(&mut self, word: &str) -> bool {
        if self.bytes[self.at..].starts_with(word.as_bytes()) {
            self.at += word.len();
            true
        } else {
            false
        }
    }

    fn value(&mut self) -> Result<Json, String> {
        self.space();
        match self.bytes.get(self.at) {
            Some(b'{') => {
                self.at += 1;
                let mut pairs = Vec::new();
                self.space();
                if self.eat("}") {
                    return Ok(Json::Object(pairs));
                }
                loop {
                    self.space();
                    if self.bytes.get(self.at) != Some(&b'"') {
                        return self.fail("expected a key");
                    }
                    let key = self.string()?;
                    self.space();
                    if !self.eat(":") {
                        return self.fail("expected :");
                    }
                    pairs.push((key, self.value()?));
                    self.space();
                    if self.eat(",") {
                        continue;
                    }
                    if self.eat("}") {
                        return Ok(Json::Object(pairs));
                    }
                    return self.fail("expected , or }");
                }
            }
            Some(b'[') => {
                self.at += 1;
                let mut items = Vec::new();
                self.space();
                if self.eat("]") {
                    return Ok(Json::List(items));
                }
                loop {
                    items.push(self.value()?);
                    self.space();
                    if self.eat(",") {
                        continue;
                    }
                    if self.eat("]") {
                        return Ok(Json::List(items));
                    }
                    return self.fail("expected , or ]");
                }
            }
            Some(b'"') => Ok(Json::Text(self.string()?)),
            Some(b't') if self.eat("true") => Ok(Json::Bool(true)),
            Some(b'f') if self.eat("false") => Ok(Json::Bool(false)),
            Some(b'n') if self.eat("null") => Ok(Json::Null),
            Some(b'-' | b'0'..=b'9') => {
                let start = self.at;
                while self.bytes.get(self.at).is_some_and(|b| b"+-.eE0123456789".contains(b)) {
                    self.at += 1;
                }
                let text = std::str::from_utf8(&self.bytes[start..self.at]).unwrap();
                text.parse().map(Json::Number).or_else(|_| self.fail("bad number"))
            }
            _ => self.fail("expected a value"),
        }
    }

    fn string(&mut self) -> Result<String, String> {
        self.at += 1;
        let mut out = String::new();
        loop {
            let start = self.at;
            while self.bytes.get(self.at).is_some_and(|&b| b != b'"' && b != b'\\') {
                self.at += 1;
            }
            out.push_str(std::str::from_utf8(&self.bytes[start..self.at]).map_err(|_| "JSON: bad UTF-8".to_string())?);
            match self.bytes.get(self.at) {
                Some(b'"') => {
                    self.at += 1;
                    return Ok(out);
                }
                Some(b'\\') => {
                    self.at += 1;
                    let escaped = *self.bytes.get(self.at).ok_or("JSON: string cut short")?;
                    self.at += 1;
                    match escaped {
                        b'n' => out.push('\n'),
                        b't' => out.push('\t'),
                        b'r' => out.push('\r'),
                        b'b' => out.push('\u{8}'),
                        b'f' => out.push('\u{c}'),
                        b'u' => {
                            let mut code = self.hex()?;
                            if (0xD800..0xDC00).contains(&code) && self.eat("\\u") {
                                let low = self.hex()?;
                                code = 0x10000 + ((code - 0xD800) << 10) + (low.wrapping_sub(0xDC00) & 0x3FF);
                            }
                            out.push(char::from_u32(code).unwrap_or('\u{FFFD}'));
                        }
                        other => out.push(other as char),
                    }
                }
                _ => return self.fail("string cut short"),
            }
        }
    }

    fn hex(&mut self) -> Result<u32, String> {
        let digits = self.bytes.get(self.at..self.at + 4).ok_or("JSON: \\u cut short")?;
        self.at += 4;
        u32::from_str_radix(std::str::from_utf8(digits).unwrap_or(""), 16).map_err(|_| "JSON: bad \\u".to_string())
    }
}

/// Write `text` as a JSON string.
pub fn quote(text: &str, out: &mut String) {
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_what_a_site_hands_in() {
        let json = r#"[{"url": "/a/", "title": "A \"b\"", "description": null, "n": -1.5e2, "ok": true,
            "sections": [{"anchor": "", "text": "café 😀"}]}]"#;
        let value = parse(json).unwrap();
        let page = &value.list().unwrap()[0];
        assert_eq!(page.get("title").unwrap().text(), Some("A \"b\""));
        assert_eq!(page.get("description").unwrap().text(), Some(""));
        assert_eq!(page.get("n"), Some(&Json::Number(-150.0)));
        let section = &page.get("sections").unwrap().list().unwrap()[0];
        assert_eq!(section.get("text").unwrap().text(), Some("café 😀"));
    }

    #[test]
    fn rejects_broken_json() {
        assert!(parse("[1, 2").is_err());
        assert!(parse("{\"a\" 1}").is_err());
        assert!(parse("[] x").is_err());
    }

    #[test]
    fn quotes_strings() {
        let mut out = String::new();
        quote("a\"b\\c\nd\u{1}", &mut out);
        assert_eq!(out, r#""a\"b\\c\nd\u0001""#);
        assert_eq!(parse(&out).unwrap(), Json::Text("a\"b\\c\nd\u{1}".into()));
    }
}
