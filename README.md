# wasm-static-search

Search any text by whatever you remember of it, instantly, in the browser. Type a line with typos, in your own words or in the wrong order, and the passage comes up as you type. Drop a folder of text on the page and it's searchable at once, or run one command and get a static folder to host. The engine is Rust, compiled to WebAssembly for the browser and to a command line tool for building, so both run the same code.

Shakespeare is the first corpus and ships as an example: "to be or not" finds Hamlet 3.1.56, and "we are such stuff as dreams are made of" finds The Tempest 4.1.156-157, where the play says "made on".

## Build it

```sh
./build.sh
```

That runs the tests, compiles the engine to WebAssembly into `web/pkg/`, builds `target/release/search-cli` (which carries the page in `web/` inside it), and builds the Shakespeare example into `target/shakespeare/site/`. It needs Rust with the `wasm32-unknown-unknown` target, plus `wasm-pack`. `web/pkg/` is committed, so `cargo build --release` alone also gives a working `search-cli`.

## Use it

Drop a folder: serve `web/` and open it. Text, Markdown, HTML and TSV files are split and indexed in the browser, and nothing leaves it.

```sh
python3 -m http.server -d web 8761
```

Host a search: split, index, bundle. Each step reads arguments or stdin and writes plain text or the index to stdout, so they compose with other tools.

```sh
search-cli split notes/ > records.tsv                  # Markdown and HTML at headings, other text at blank lines
search-cli index records.tsv > index.bin               # add --table FILE for each table
search-cli search index.bin "the line you remember"    # rank, ref, score and text as TSV
search-cli bundle index.bin -o site --name Notes       # a static page, engine, index, service worker and icons
search-cli offline site --name Notes                   # a service worker, manifest and icons for a built folder
```

`split --by line|paragraph|heading` overrides how files are cut. `bundle` takes `--about` and `--credit` for the page's lines of text, `--theme` and `--css` for its look (see Themes), and `--file` for anything to ship beside the page, such as a license. Gzipping `site/index.bin` in place shrinks the download, and the page unzips it itself. `eval INDEX CASES...` scores a set of queries: each line is a query, a tab, and the refs that count, joined by `|`.

In a search, `speaker:juliet` keeps records whose `speaker` field holds that word, `in:hamlet` keeps one name's records, and `"quoted words"` must appear as written. A ref goes straight to its record: "hamlet 3.1.56", "ham 3.1", "ham 3.1.56-60" or "sonnet 18".

## Put it in any page

The search is one element, `<text-search>`. A folder from `search-cli bundle` holds everything it needs, and any page on the same site can use it. Workers have to come from the page's own origin, so the folder can't sit on another host.

```html
<link rel="stylesheet" href="/search/style.css">
<script type="module" src="/search/app.js"></script>
<text-search></text-search>
```

An empty element fills in a search box, a status line and the results. To write the markup yourself, give it a `<form>` with an `<input type="search">`, an `<output>` and an `<ol>`, plus anything else you want around them. A search box with a `name` keeps the search in the page's address under that name, and one with `autofocus` takes the cursor once the index loads. `/` focuses the box from anywhere on the page outside a text field. `j`, `k`, `Enter`, `o` and `Escape` work only while focus is inside the element.

## Themes

With no `theme` the element has no look of its own. It takes the page's font and text color, and makes its lines, highlights and quiet text from that color, so it fits a light page or a dark panel alike. A theme gives it one:

| Theme | Look |
|---|---|
| `paper` | Cream paper, serif type, a red accent. The bundled page's default. |
| `swiss` | Black on white in Helvetica, an underlined search box, found words in bold. |
| `system` | The browser's system colors and the system font, so it looks native on every device. |

```html
<text-search theme="swiss"></text-search>
```

Each theme has a light side and a dark side. The element follows whichever `color-scheme` it inherits, and `text-search { color-scheme: dark }` holds it dark. The bundled page follows the device.

Every rule in `style.css` sits in a cascade layer (`@layer text-search`) and inside `:where()`, so any rule on your page wins, with no `!important` and no specificity contest. To change one thing, set a token on the element itself. Setting it on `:root` doesn't work, because the element declares its own.

```css
text-search { --ts-accent: teal; --ts-font: "Inter", sans-serif; --ts-width: 50rem; }
```

| Token | Sets |
|---|---|
| `--ts-paper`, `--ts-ink` | background and text |
| `--ts-quiet` | the status line, a result's place, the footer |
| `--ts-rule` | lines between results |
| `--ts-accent` | a result's reference, focus rings, links |
| `--ts-mark`, `--ts-found`, `--ts-found-weight` | the background, color and weight of found words |
| `--ts-here` | behind the found line in an opened result |
| `--ts-font`, `--ts-font-ui`, `--ts-font-code` | body type; small type (status, references, footer); keys and code |
| `--ts-size`, `--ts-small`, `--ts-leading` | body size, small size, line height |
| `--ts-width`, `--ts-radius` | the widest the content runs, and corner rounding |
| `--ts-box-border`, `--ts-box-line`, `--ts-box-padding`, `--ts-box-size`, `--ts-box-weight`, `--ts-box-ring` | the search box: border widths (`0 0 1px` underlines it), border color, padding, type size and weight, and focus outline |

A token set to `initial` hands that property back to the page around the element. For a whole theme of your own, write the tokens in a stylesheet and pass it to `bundle --css FILE`. It ships beside the page and loads after `style.css`, so every rule in it wins. `bundle --theme NAME` picks a built-in theme and refuses a name `style.css` doesn't define.

## Offline

A bundled search works offline after one visit, and can be installed: Chrome puts an install button in the address bar, and Safari on iOS has Add to Home Screen. `search-cli bundle` writes what that takes, and `search-cli offline DIR` writes the same into any folder that's already built, such as a page you made another way:

- `sw.js`, a service worker listing every file in the folder.
- `manifest.webmanifest`, with the page's name and description from `--name` and `--about`.
- Icons in `icons/`: a magnifying glass, or a globe with `--icon globe`.

Once the page is ready, the worker saves every file it lists, so the first visit downloads everything once. Files the browser already loaded come back from the server as 304s, so they aren't downloaded twice. From then on the page and its files come from that saved copy, with or without a network.

The worker's cache name holds a hash of every file it lists, so a rebuild that changes any file writes a new `sw.js`. The first visit after the rebuild still shows the old copy while the browser saves the new one in the background and deletes the old. The next visit shows the new one. Building twice from the same data gives the same files and the same hash, so visitors download nothing new.

Service workers run only on https and localhost. The bundled page registers its worker on https only, so editing `web/` locally never shows a stale copy.

To clear the saved copy in Chrome, open DevTools on the page and use Application, Storage, Clear site data, or delete the site's data under Settings, Privacy and security, Site settings, View permissions and data stored across sites. An installed copy uninstalls from its own window's menu.

## Records and tables

Records are tab-separated text under a header line:

```text
#ref	text	speaker	kind
Hamlet 3.1.56	To be, or not to be, that is the question:	Hamlet	verse
```

- `ref` and `text` are required. `heading` and `anchor` place a text under a heading. `boost` is a number that multiplies a record's score by 1 + boost, so among records matching a query equally well the better known comes first, such as a famous place over an obscure one. Every other column is a field, shown with results and usable as a filter. `title*2` searches a field with each word counting twice, and `title^2` adds twice a word's rarity when the field holds it. `region+1` counts a field's words only for a record the query's other words found, so "portland oregon" narrows to Oregon while "oregon" alone finds only what's named Oregon. A place search can use it for a place's state and country. A `link` column holding an http or https address puts a link to it under the result, and `o` opens it from the keyboard.
- A ref is a name and then numbers joined by `.` or `:` ("Hamlet 3.1.56", "Psalms 23:1"). Records with the same name and every number but the last are neighbours, so a passage can run across them. A ref without numbers is a key, like a headword.
- In a cell, `\t`, `\n` and `\\` stand for a tab, a line break and a backslash.

A corpus can bring tables, each tab-separated, a key and then its values, with `#` for comments. The file name says which table it is:

| Table | Holds |
|---|---|
| `forms.tsv` | Irregular and old forms and the word they belong to: spake → speak, thou → you. A word mapped to itself is kept from the suffix rules. |
| `suffixes.tsv` | Endings rewritten before stemming: -eth → -s, -'d → -ed. |
| `synonyms.tsv` | Words that say the same thing, each with an optional `:weight` from 0 to 100. |
| `aliases.tsv` | A ref name, then the other names it goes by: Hamlet → Ham. |
| `stop.tsv` | Words that add to a match but never make one. |

## How ranking works

- **Words.** Lower case, accents removed, and the apostrophe inside a word dropped, so "don't" is "dont". Every word is indexed, common ones included.
- **What a query word matches.** Itself; its other forms (plural, `-ed` and `-ing`, plus the corpus's forms and suffixes) at 0.9; its synonyms at 0.6 times their weight; and, for the last word while typing or a word found nowhere, the words it begins, at 0.3 to 0.8 by how much is typed. While typing, a word that begins too many others keeps only the commoner ones. A word with none of these is taken as a typo and matches words one edit away (four to seven letters) or two (eight or more).
- **Score.** BM25 with `k1 = 1.2` and `b = 0.4`, with weighted fields and headings counting extra.
- **Order.** Results holding more of the query's content words come first. Among those, a record's `boost` multiplies its score, and every record of a key comes in order of boost. Among the best 300, a record gains up to double when it keeps the query's neighbouring words side by side, each pair counted by its rarer word, and the gain shrinks for texts past 40 words.
- **Passages.** Up to four neighbouring records make one result when together they hold more of the query than any one of them does. Each record past the first costs a fifth.
- **Snippet.** The 16-word run holding the most of the query, or a heading that matches, widened to about 150 bytes.

## Searching a site's pages

A static site can hand its pages to `load_pages(json)`, an array of `{url, title, description, sections: [{anchor, heading, text}]}`, and calls `search(query, 8)`. Results are `{url, title, heading, anchor, snippet, find, score}`, plus `ref`, `fields` and `more` for passages. A build for this leaves out loading index files and splitting files:

```sh
wasm-pack build crates/wasm --release --target web --out-dir "$PWD/pkg" -- --no-default-features
```

## Numbers (October 2026, M-series Mac, Chrome 152)

- Shakespeare download: 2.6 MB for the gzipped index of all 111,581 lines and stage directions, and 0.2 MB for the wasm.
- Shakespeare load: ready 340 ms after the page opens from a local server, of which 260 to 310 ms is building the index. The command line tool reads the index in 69 ms and builds it in about 200 ms.
- Shakespeare search: 0.7 ms median and 2.2 ms slowest for the 68 whole queries in both sets. Every keystroke of those queries, 2,343 in all, takes 0.5 ms median, 2.7 ms at the 95th percentile and 4.1 ms at most.
- Half-remembered lines (`examples/shakespeare/`): 38 of 38 at rank 1 on the tuned set. On the held-out set, written before any tuning and never tuned against, 28 of 30 at rank 1, 29 of 30 in the top 3 and 30 of 30 in the top 5. The tuned set needed no ranking changes. Later changes for speed and for page search moved one held-out line, "what a piece of work is man", from third to fourth.
- Page search with `load_pages`: 155 KB of wasm, and 112 pages index in 6 ms.

## Layout

- `crates/core/`: the engine. `text.rs` splits words and stems them, `tables.rs` reads a corpus's tables, `records.rs` reads records and index files, `index.rs` builds and searches, `split.rs` turns files into records, and `pages.rs` takes a site's pages.
- `crates/cli/`: `search-cli`, and in `icons.rs` the home-screen icons it draws as PNGs.
- `crates/wasm/`: the browser's engine: `load_pages`, `load_index`, `index_files`, `search`, `around` and `meta`.
- `web/`: the `<text-search>` element (`app.js`), its themes (`style.css`), the bundled page around it, its worker and service worker, and the engine in `web/pkg/`.
- `examples/shakespeare/`: `prepare.py` fetches the PlayShakespeare.com XML pinned to one commit and writes `records.tsv` and `aliases.tsv`. The other tables and both query sets are written by hand. `build.sh` indexes it, bundles it and scores it.

## Sources and licenses

- The code is MIT.
- The Shakespeare text is the PlayShakespeare.com modern edition, 5.4, from github.com/severdia/PlayShakespeare.com-XML at commit 9168116, under the GNU Free Documentation License 1.3 or later, whose text is `examples/shakespeare/fdl-1.3.txt` and ships with the page. `prepare.py` downloads it into `examples/shakespeare/raw/`, which is not committed.

## Outside this folder

Nothing. Building writes only to `target/` and `web/pkg/`.
