//! search-cli: split text into records, index them, search the index, score it, bundle it as a page, and make a
//! folder of a page work offline.

mod icons;

use std::io::{self, BufRead, IsTerminal, Read, Write};
use std::path::{Path, PathBuf};
use std::time::Instant;
use std::{env, fs, process};

use search_core::index::Index;
use search_core::records::Corpus;
use search_core::split::{self, By};
use search_core::tables;

use icons::Icon;

const USAGE: &str = "\
usage:
  search-cli split [--by line|paragraph|heading] FILE|DIR...   records as TSV to stdout
  search-cli index [--table FILE]... [RECORDS]                 an index to stdout; RECORDS defaults to stdin
  search-cli search INDEX [QUERY...]                           rank, ref, score and text as TSV; queries from stdin if none
  search-cli eval INDEX CASES...                               each case's rank, then a summary per file
  search-cli bundle INDEX -o DIR [--name N] [--about T] [--credit T] [--theme T] [--css F] [--file F]...
                                                               a static search page in DIR
  search-cli offline DIR [--name N] [--about T] [--icon search|globe] [--lib LIB=URL] [--later FOLDER,...]
                                                               a service worker, manifest and icons for DIR, also saving LIB, served at URL

A table's file name says what it is: forms.tsv, suffixes.tsv, synonyms.tsv, aliases.tsv or stop.tsv.";

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("split") => split_files(&args[1..]),
        Some("index") => index(&args[1..]),
        Some("search") => search(&args[1..]),
        Some("eval") => eval(&args[1..]),
        Some("bundle") => bundle(&args[1..]),
        Some("offline") => offline(&args[1..]),
        Some("-h" | "--help" | "help") => {
            println!("{USAGE}");
            Ok(())
        }
        _ => Err(USAGE.to_string()),
    };
    if let Err(message) = result {
        // A reader that stopped early, like head, is not a failure.
        if message.contains("Broken pipe") {
            return;
        }
        eprintln!("{message}");
        process::exit(1);
    }
}

/// Pull `--flag value` out of the arguments, leaving the rest.
fn take(args: &mut Vec<String>, flag: &str) -> Result<Option<String>, String> {
    let Some(at) = args.iter().position(|a| a == flag) else { return Ok(None) };
    if at + 1 >= args.len() {
        return Err(format!("{flag} needs a value"));
    }
    let value = args.remove(at + 1);
    args.remove(at);
    Ok(Some(value))
}

fn read(path: &Path) -> Result<String, String> {
    fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))
}

fn split_files(args: &[String]) -> Result<(), String> {
    let mut args = args.to_vec();
    let by = match take(&mut args, "--by")? {
        Some(word) => Some(By::parse(&word).ok_or(format!("--by takes line, paragraph or heading, not {word}"))?),
        None => None,
    };
    if args.is_empty() {
        return Err("split: name the files or folders to split".into());
    }
    let mut paths = Vec::new();
    for arg in &args {
        walk(Path::new(arg), &mut paths)?;
    }
    let (records, texts): (Vec<PathBuf>, Vec<PathBuf>) = paths.into_iter().partition(|p| p.extension().is_some_and(|e| e == "tsv"));
    let corpus = if !records.is_empty() {
        if !texts.is_empty() {
            return Err("split: a .tsv is already records; split the other files on their own, or give the .tsv to index".into());
        }
        let mut all = Corpus::default();
        for path in &records {
            let one = Corpus::from_tsv(&read(path)?).map_err(|e| format!("{}: {e}", path.display()))?;
            if !all.records.is_empty() && one.fields != all.fields {
                return Err(format!("{}: its columns differ from the files before it", path.display()));
            }
            all.fields = one.fields;
            all.records.extend(one.records);
        }
        all
    } else {
        let mut files = Vec::new();
        for path in &texts {
            match fs::read_to_string(path) {
                Ok(text) => files.push((path.to_string_lossy().into_owned(), text)),
                Err(_) => eprintln!("split: skipped {}, not text", path.display()),
            }
        }
        split::corpus(&files, by)
    };
    io::stdout().write_all(corpus.to_tsv().as_bytes()).map_err(|e| e.to_string())
}

/// Every file under a path, in name order, leaving out hidden ones.
fn walk(path: &Path, out: &mut Vec<PathBuf>) -> Result<(), String> {
    if path.is_dir() {
        let mut entries: Vec<PathBuf> = fs::read_dir(path).map_err(|e| format!("{}: {e}", path.display()))?.flatten().map(|e| e.path()).collect();
        entries.sort();
        for entry in entries {
            if !entry.file_name().is_some_and(|n| n.to_string_lossy().starts_with('.')) {
                walk(&entry, out)?;
            }
        }
    } else if path.exists() {
        out.push(path.to_path_buf());
    } else {
        return Err(format!("{}: no such file", path.display()));
    }
    Ok(())
}

fn index(args: &[String]) -> Result<(), String> {
    let mut args = args.to_vec();
    let mut corpus_tables = Vec::new();
    while let Some(path) = take(&mut args, "--table")? {
        corpus_tables.push(PathBuf::from(path));
    }
    let source = match args.as_slice() {
        [] => {
            let mut text = String::new();
            io::stdin().read_to_string(&mut text).map_err(|e| format!("stdin: {e}"))?;
            text
        }
        [path] => read(Path::new(path))?,
        _ => return Err("index: one records file, or records on stdin".into()),
    };
    let mut corpus = Corpus::from_tsv(&source)?;
    for path in &corpus_tables {
        let name = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        if !tables::NAMES.contains(&name.as_str()) {
            return Err(format!("{}: a table's file name must be one of {}", path.display(), tables::NAMES.map(|n| format!("{n}.tsv")).join(", ")));
        }
        corpus.tables.add(&name, &read(path)?).map_err(|e| format!("{}: {e}", path.display()))?;
    }
    if corpus.records.is_empty() {
        return Err("index: no records".into());
    }
    let bytes = corpus.to_bytes();
    if io::stdout().is_terminal() {
        return Err("index: the index is for a file; send stdout to one, as in search-cli index records.tsv > index.bin".into());
    }
    io::stdout().write_all(&bytes).map_err(|e| e.to_string())?;
    eprintln!("{} records, {} tables, {} bytes", corpus.records.len(), corpus.tables.sources.len(), bytes.len());
    Ok(())
}

/// An index file, read and built, with how long each took in milliseconds.
fn load(path: &str) -> Result<(Index, f64, f64), String> {
    let bytes = fs::read(path).map_err(|e| format!("{path}: {e}"))?;
    let started = Instant::now();
    let corpus = Corpus::from_bytes(&bytes).map_err(|e| format!("{path}: {e}"))?;
    let read = started.elapsed().as_secs_f64() * 1000.0;
    let index = Index::new(corpus);
    Ok((index, read, started.elapsed().as_secs_f64() * 1000.0 - read))
}

/// A result's text, its records joined by " / ".
fn text_of(hit: &search_core::index::Hit) -> String {
    hit.shown.iter().map(|s| s.snippet.iter().map(|p| p.0.as_str()).collect::<String>()).collect::<Vec<_>>().join(" / ")
}

fn search(args: &[String]) -> Result<(), String> {
    let Some(path) = args.first() else { return Err("search: name the index".into()) };
    let (index, _, _) = load(path)?;
    let queries: Vec<String> = if args.len() > 1 {
        vec![args[1..].join(" ")]
    } else {
        io::stdin().lock().lines().map_while(Result::ok).collect()
    };
    let mut out = io::stdout().lock();
    for (i, query) in queries.iter().enumerate() {
        if i > 0 {
            writeln!(out).map_err(|e| e.to_string())?;
        }
        // A whole query is finished; the last word is not still being typed.
        for (rank, hit) in index.search(&format!("{query} "), 10).iter().enumerate() {
            let span = index.span(hit.shown[0].doc, hit.shown.last().unwrap().doc);
            writeln!(out, "{}\t{span}\t{:.2}\t{}", rank + 1, hit.score, text_of(hit)).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

fn eval(args: &[String]) -> Result<(), String> {
    let Some((path, cases)) = args.split_first() else { return Err("eval: name the index and the cases".into()) };
    if cases.is_empty() {
        return Err("eval: name a cases file: query, a tab, then the refs that count, joined by |".into());
    }
    let (index, read_ms, build_ms) = load(path)?;
    let mut out = io::stdout().lock();
    let e = |e: io::Error| e.to_string();
    writeln!(out, "{path}: {} records, read in {read_ms:.0} ms and indexed in {build_ms:.0} ms", index.len()).map_err(e)?;
    for file in cases {
        let mut ranks = Vec::new();
        let mut slowest = 0u128;
        writeln!(out, "\n{file}").map_err(e)?;
        for (i, line) in read(Path::new(file))?.lines().enumerate() {
            if line.starts_with('#') || line.trim().is_empty() {
                continue;
            }
            let (query, wanted) = line.split_once('\t').ok_or(format!("{file} line {}: want a query, a tab, and refs", i + 1))?;
            let wanted: Vec<&str> = wanted.split('|').map(str::trim).collect();
            let started = Instant::now();
            let hits = index.search(&format!("{query} "), 10);
            let micros = started.elapsed().as_micros();
            slowest = slowest.max(micros);
            let rank = hits.iter().position(|h| h.shown.iter().any(|s| wanted.contains(&index.reference(s.doc)))).map(|r| r + 1);
            let top = hits.first().map_or(String::from("nothing"), |h| index.span(h.shown[0].doc, h.shown.last().unwrap().doc));
            let mark = rank.map_or(String::from(" miss"), |r| format!("  #{r:<2}"));
            let short: String = query.chars().take(50).collect();
            writeln!(out, "{mark} {micros:>6} µs  {short:<50}  top: {top}").map_err(e)?;
            ranks.push(rank);
        }
        let within = |n: usize| ranks.iter().filter(|r| r.is_some_and(|r| r <= n)).count();
        writeln!(
            out,
            "{file}: {} of {} at rank 1, {} in the top 3, {} in the top 5; slowest {slowest} µs",
            within(1),
            ranks.len(),
            within(3),
            within(5)
        )
        .map_err(e)?;
    }
    Ok(())
}

const STYLE: &str = include_str!("../../../web/style.css");

/// The themes style.css defines, each as `[theme="name"]`.
fn themes() -> Vec<&'static str> {
    STYLE.split("[theme=\"").skip(1).filter_map(|rest| rest.split('"').next()).collect()
}

fn file_name(path: &Path) -> Result<String, String> {
    Ok(path.file_name().ok_or(format!("{}: not a file", path.display()))?.to_string_lossy().into_owned())
}

/// The page, its scripts and its engine, built into this program so a bundle needs nothing else.
const WEB: &[(&str, &[u8])] = &[
    ("index.html", include_bytes!("../../../web/index.html")),
    ("style.css", STYLE.as_bytes()),
    ("app.js", include_bytes!("../../../web/app.js")),
    ("worker.js", include_bytes!("../../../web/worker.js")),
    ("sw.js", include_bytes!("../../../web/sw.js")),
    ("pkg/search_wasm.js", include_bytes!("../../../web/pkg/search_wasm.js")),
    ("pkg/search_wasm_bg.wasm", include_bytes!("../../../web/pkg/search_wasm_bg.wasm")),
];

fn bundle(args: &[String]) -> Result<(), String> {
    let mut args = args.to_vec();
    let dir = PathBuf::from(take(&mut args, "-o")?.ok_or("bundle: say where with -o DIR")?);
    let name = take(&mut args, "--name")?;
    let about = take(&mut args, "--about")?;
    let credit = take(&mut args, "--credit")?.unwrap_or_default();
    let theme = take(&mut args, "--theme")?.unwrap_or("paper".into());
    if !STYLE.contains(&format!("[theme=\"{theme}\"]")) {
        return Err(format!("bundle: no theme called {theme}; there's {}", themes().join(", ")));
    }
    let css = take(&mut args, "--css")?.map(PathBuf::from);
    let mut extra = Vec::new();
    while let Some(file) = take(&mut args, "--file")? {
        extra.push(PathBuf::from(file));
    }
    let [path] = args.as_slice() else { return Err("bundle: name one index".into()) };
    let index_bytes = fs::read(path).map_err(|e| format!("{path}: {e}"))?;
    let records = Corpus::from_bytes(&index_bytes).map_err(|e| format!("{path}: {e}"))?.records.len();
    let name = name.unwrap_or_else(|| Path::new(path).file_stem().map_or("Search".into(), |s| s.to_string_lossy().into_owned()));
    let about = about.unwrap_or_else(|| format!("Search {name} by whatever you remember, as you type, in your browser."));

    let mut files: Vec<(String, Vec<u8>)> = Vec::new();
    for &(file, bytes) in WEB {
        let mut bytes = bytes.to_vec();
        if file == "index.html" {
            let page = String::from_utf8(bytes).unwrap()
                .replace("<title>Search any text</title>", &format!("<title>{}</title>", html(&name)))
                .replace("<a href=\"./\">Search any text</a>", &format!("<a href=\"./\">{}</a>", html(&name)))
                .replace(
                    "Drop a folder of text and search it by whatever you remember, as you type, in your browser.\">",
                    &format!("{}\">", html(&about)),
                )
                .replace(
                    "<p>Drop a folder of text and search it by whatever you remember, as you type, in your browser.</p>",
                    &format!("<p>{}</p>", html(&about)),
                )
                .replace("<text-search theme=\"paper\">", &format!("<text-search theme=\"{}\">", html(&theme)));
            let page = if credit.is_empty() { page } else { page.replace("<footer>\n", &format!("<footer>\n    <p>{}</p>\n", html(&credit))) };
            // A page's own stylesheet comes after the engine's, which sits in a layer, so every rule in it wins.
            let page = match &css {
                Some(path) => page.replace(
                    "<script type=\"module\"",
                    &format!("<link rel=\"stylesheet\" href=\"{}\">\n<script type=\"module\"", html(&file_name(path)?)),
                ),
                None => page,
            };
            bytes = page.into_bytes();
        }
        if file != "sw.js" {
            files.push((file.to_string(), bytes));
        }
    }
    files.push(("index.bin".into(), index_bytes));
    // Files to ship beside the page, such as the license its text is under.
    for path in extra.iter().chain(&css) {
        files.push((file_name(path)?, fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?));
    }
    for (file, bytes) in &files {
        let at = dir.join(file);
        fs::create_dir_all(at.parent().unwrap()).map_err(|e| format!("{}: {e}", dir.display()))?;
        fs::write(&at, bytes).map_err(|e| format!("{}: {e}", at.display()))?;
    }
    let names: Vec<String> = files.iter().map(|(f, _)| f.clone()).collect();
    write_offline(&dir, &names, &[], &[], &[], &name, &about, Icon::Search)?;
    let total: usize = files.iter().map(|(_, b)| b.len()).sum();
    eprintln!("{} records, {} files, {total} bytes in {}", records, files.len(), dir.display());
    Ok(())
}

/// `search-cli offline DIR`: make a built page installable and cached, listing every file in DIR.
fn offline(args: &[String]) -> Result<(), String> {
    let mut args = args.to_vec();
    let name = take(&mut args, "--name")?;
    let about = take(&mut args, "--about")?.unwrap_or_default();
    let icon = match take(&mut args, "--icon")? {
        Some(i) => Icon::named(&i).ok_or(format!("offline: no icon called {i}; there's search and globe"))?,
        None => Icon::Search,
    };
    let lib = take(&mut args, "--lib")?;
    // Folders whose files are saved the first time the page fetches them, not on install, like map tiles a visitor
    // may never look at.
    let later: Vec<String> = take(&mut args, "--later")?.map(|l| l.split(',').map(|f| format!("{}/", f.trim_end_matches('/'))).collect()).unwrap_or_default();
    let [dir] = args.as_slice() else { return Err("offline: name one folder".into()) };
    let dir = PathBuf::from(dir);
    if !dir.is_dir() {
        return Err(format!("{}: not a folder", dir.display()));
    }
    let name = name.unwrap_or_else(|| dir.canonicalize().ok().and_then(|d| d.file_name().map(|n| n.to_string_lossy().into_owned())).unwrap_or("Search".into()));
    let mut found = Vec::new();
    walk(&dir, &mut found)?;
    let (mut files, mut later_files) = (Vec::new(), Vec::new());
    for path in found {
        let file = path.strip_prefix(&dir).unwrap().components().map(|c| c.as_os_str().to_string_lossy()).collect::<Vec<_>>().join("/");
        // Last time's worker, manifest and icons get written again below.
        if file != "sw.js" && file != "manifest.webmanifest" && !file.starts_with("icons/") && !file.ends_with(".md") {
            if later.iter().any(|f| file.starts_with(f.as_str())) { later_files.push(file) } else { files.push(file) }
        }
    }
    let lib = match lib {
        Some(l) => {
            let (from, url) = l.split_once('=').ok_or("offline: --lib takes LIB=URL")?;
            let mut found = Vec::new();
            walk(Path::new(from), &mut found)?;
            found.iter().map(|p| {
                let rel = p.strip_prefix(from).unwrap().components().map(|c| c.as_os_str().to_string_lossy()).collect::<Vec<_>>().join("/");
                (format!("{}/{rel}", url.trim_end_matches('/')), p.clone())
            }).collect()
        }
        None => Vec::new(),
    };
    let files = write_offline(&dir, &files, &lib, &later, &later_files, &name, &about, icon)?;
    let total: u64 = files.iter().filter_map(|f| fs::metadata(dir.join(f)).ok()).map(|m| m.len()).sum();
    eprintln!("{} files, {total} bytes, cached by {}", files.len(), dir.join("sw.js").display());
    if !later.is_empty() {
        let size: u64 = later_files.iter().filter_map(|f| fs::metadata(dir.join(f)).ok()).map(|m| m.len()).sum();
        eprintln!("{} more files, {size} bytes, in {}, cached as they're fetched", later_files.len(), later.join(" "));
    }
    Ok(())
}

/// Write a manifest and icons into DIR, then a service worker that saves them and `files`, all already in DIR, on the
/// first visit, and saves files under the `later` folders as they're fetched. Its cache name is a hash of every file
/// it may save, `later_files` included, so changing any of them replaces the cache. Returns the files it saves up front.
/// `lib` adds files from outside DIR, each with the address it's served at and where it is on disk.
#[allow(clippy::too_many_arguments)]
fn write_offline(dir: &Path, files: &[String], lib: &[(String, PathBuf)], later: &[String], later_files: &[String], name: &str, about: &str, icon: Icon) -> Result<Vec<String>, String> {
    let mut extra: Vec<(String, Vec<u8>)> = vec![
        ("manifest.webmanifest".into(), manifest(name, about, icon).into_bytes()),
        ("icons/icon.svg".into(), icon.svg().as_bytes().to_vec()),
    ];
    for size in [180, 192, 512] {
        extra.push((format!("icons/icon-{size}.png"), icon.png(size)));
    }
    for (file, bytes) in &extra {
        let at = dir.join(file);
        fs::create_dir_all(at.parent().unwrap()).map_err(|e| format!("{}: {e}", dir.display()))?;
        fs::write(&at, bytes).map_err(|e| format!("{}: {e}", at.display()))?;
    }
    let all: Vec<String> = files.iter().chain(extra.iter().map(|(f, _)| f)).cloned().collect();

    let mut hash: u64 = 0xcbf29ce484222325;
    for file in all.iter().chain(later_files) {
        let bytes = fs::read(dir.join(file)).map_err(|e| format!("{}: {e}", dir.join(file).display()))?;
        for b in bytes {
            hash = (hash ^ b as u64).wrapping_mul(0x100000001b3);
        }
    }
    for (_, path) in lib {
        let bytes = fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
        for b in bytes {
            hash = (hash ^ b as u64).wrapping_mul(0x100000001b3);
        }
    }
    let list: Vec<String> = std::iter::once("\"./\"".to_string()).chain(all.iter().map(|f| format!("\"{f}\""))).chain(lib.iter().map(|(u, _)| format!("\"{u}\""))).collect();
    let sw = String::from_utf8(WEB.iter().find(|(f, _)| *f == "sw.js").unwrap().1.to_vec()).unwrap()
        .replace("const VERSION = \"dev\";", &format!("const VERSION = \"{:012x}\";", hash & 0xffff_ffff_ffff))
        .replace("const FILES = [\"./\"];", &format!("const FILES = [{}];", list.join(", ")))
        .replace("const LATER = [];", &format!("const LATER = [{}];", later.iter().map(|f| format!("\"{f}\"")).collect::<Vec<_>>().join(", ")));
    fs::write(dir.join("sw.js"), sw).map_err(|e| format!("{}: {e}", dir.join("sw.js").display()))?;
    Ok(all)
}

fn manifest(name: &str, about: &str, icon: Icon) -> String {
    let mut quoted = (String::new(), String::new());
    search_core::json::quote(name, &mut quoted.0);
    search_core::json::quote(about, &mut quoted.1);
    format!(
        r##"{{
  "name": {name},
  "short_name": {name},
  "description": {about},
  "start_url": ".",
  "scope": ".",
  "display": "standalone",
  "background_color": {paper},
  "theme_color": {paper},
  "icons": [
    {{ "src": "icons/icon-192.png", "sizes": "192x192", "type": "image/png", "purpose": "any maskable" }},
    {{ "src": "icons/icon-512.png", "sizes": "512x512", "type": "image/png", "purpose": "any maskable" }},
    {{ "src": "icons/icon.svg", "sizes": "any", "type": "image/svg+xml" }}
  ]
}}
"##,
        name = quoted.0,
        about = quoted.1,
        paper = format!("\"{}\"", icon.paper())
    )
}

fn html(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}
