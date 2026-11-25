// <text-search>: a search box, results as you type, and the lines around a result when it's opened. Any page can hold
// one: load this script from the folder `search-cli bundle` wrote, on the same site, and the element searches that
// folder's index. An empty element fills in a search box, a status line and a list of results. The bundled page writes
// its own markup, with a header, a footer, and a section asking for files that shows when there's no index; then a
// folder dropped anywhere on the page is searched instead.
//
// The script can also live apart from the index, shared by many pages: `index` names the index, and `offline` names a
// service worker, both relative to the page. `search-cli offline DIR --lib LIB=URL` writes one that saves this script too.
const TEXT = /\.(txt|text|md|markdown|html?|tsv)$/i;
const HERE = new URL(".", import.meta.url).href;
const FILLING = `<form role="search"><input type="search" aria-label="Search" autocomplete="off" autocapitalize="off"
  spellcheck="false" enterkeyhint="search" placeholder="Search" disabled></form><output></output><ol></ol>`;

// Fields shown beside a result. kind and is hold filter words, title is shown already, and a link gets its own line.
const UNSHOWN = new Set(["kind", "is", "title", "link"]);

customElements.define("text-search", class extends HTMLElement {
  connectedCallback() {
    if (this.started) return;
    this.started = true;
    start(this, this.attachInternals());
  }
});

function start(root, internals) {
  if (!root.querySelector("input[type=search]")) root.insertAdjacentHTML("beforeend", FILLING);
  const box = root.querySelector("input[type=search]");
  const results = root.querySelector("ol");
  const status = root.querySelector("output");
  const drop = root.querySelector("section:has(input[type=file])");
  const workerUrl = new URL("worker.js", import.meta.url).href;
  const ownIndex = root.getAttribute("index");
  // A shared script sits outside the page's service worker's folder. A worker started from a blob stays with the page,
  // so its files still come from the offline copy. Its name carries the index.
  const worker = ownIndex
    ? new Worker(URL.createObjectURL(new Blob([`import ${JSON.stringify(workerUrl)};`], { type: "text/javascript" })),
                 { type: "module", name: new URL(ownIndex, document.baseURI).href })
    : new Worker(workerUrl, { type: "module" });

  let latest = 0;
  let records = 0;

  function say(text) {
    status.textContent = text;
  }

  // The offline copy starts once the page's own downloads are done, so it saves what the browser already holds. Only
  // a page inside this folder registers it, since a service worker only serves pages under its own folder.
  function keep() {
    if (!("serviceWorker" in navigator) || location.protocol !== "https:") return;
    const offline = root.getAttribute("offline");
    if (offline) navigator.serviceWorker.register(new URL(offline, document.baseURI));
    else if (location.href.startsWith(HERE)) navigator.serviceWorker.register(new URL("sw.js", import.meta.url));
  }

  function ready(meta, ms, verb) {
    records = meta.records;
    box.disabled = false;
    if (drop) drop.hidden = true;
    say(`${verb} ${records.toLocaleString()} records in ${Math.round(ms)} ms.`);
    const q = box.name && new URLSearchParams(location.search).get(box.name);
    if (q && !box.value) box.value = q;
    if (box.value) find();
    if (box.autofocus && !matchMedia("(pointer: coarse)").matches) box.focus();
  }

  worker.onmessage = (e) => {
    const m = e.data;
    if (m.type === "ready") {
      if (m.meta) {
        ready(m.meta, m.ms, "Loaded");
        keep();
      } else {
        keep();
        if (drop) drop.hidden = false;
        say(drop ? "Nothing to search yet." : `Nothing to search: no ${ownIndex || "index.bin beside app.js"}.`);
      }
    } else if (m.type === "indexed") {
      if (m.count > 0) ready(m.meta, m.ms, "Indexed");
      else say("No text in those files.");
    } else if (m.type === "results" && m.id === latest) {
      show(m.results, m.ms);
    } else if (m.type === "around") {
      const li = results.querySelector(`li[data-id="${m.id}"]`);
      if (li) context(li, m.lines);
    } else if (m.type === "error") {
      say(`Couldn't load the search: ${m.message}`);
    }
  };

  // A box with a name keeps its search in the address, under that name, so a search can be shared.
  function find() {
    const q = box.value;
    if (box.name) {
      const url = new URL(location.href);
      if (q.trim()) url.searchParams.set(box.name, q);
      else url.searchParams.delete(box.name);
      history.replaceState(history.state, "", url);
    }
    if (!q.trim()) {
      results.replaceChildren();
      say(records ? `${records.toLocaleString()} records.` : "");
      return;
    }
    worker.postMessage({ type: "search", id: ++latest, q, max: 20 });
  }

  function marked(snippet) {
    const out = document.createElement("span");
    for (const [text, hit] of snippet) out.append(hit ? Object.assign(document.createElement("mark"), { textContent: text }) : text);
    return out;
  }

  function label(fields) {
    return Object.entries(fields)
      .filter(([name, value]) => value && !UNSHOWN.has(name) && !/^https?:\/\//.test(value))
      .map(([, value]) => value)
      .join(" · ");
  }

  function show(hits, ms) {
    say(hits.length ? `${hits.length === 20 ? "Top 20" : hits.length} in ${ms < 1 ? ms.toFixed(2) : Math.round(ms)} ms.` : "Nothing matches. Try other words.");
    results.replaceChildren(...hits.map((hit, i) => {
      const li = document.createElement("li");
      li.dataset.id = `${latest}-${i}`;
      li.dataset.ref = hit.url;
      if (/^https?:\/\//.test(hit.fields.link || "")) li.dataset.link = hit.fields.link;
      const button = li.appendChild(document.createElement("button"));
      button.type = "button";
      button.setAttribute("aria-expanded", "false");
      const cite = button.appendChild(document.createElement("cite"));
      cite.textContent = hit.ref;
      const about = [...new Set([hit.title !== hit.url && hit.title, hit.heading, label(hit.fields)].filter(Boolean))].join(" · ");
      if (about) cite.append(" ", Object.assign(document.createElement("span"), { textContent: about }));
      button.append(marked(hit.snippet), ...hit.more.map((m) => marked(m.snippet)));
      // Safari leaves a clicked button unfocused, and the keys only work from inside the element.
      button.onclick = () => {
        button.focus();
        open(li, button);
      };
      if (li.dataset.link) {
        const a = li.appendChild(document.createElement("a"));
        a.href = li.dataset.link;
        a.target = "_blank";
        a.rel = "noopener";
        a.textContent = `Open on ${li.dataset.link.replace(/^https?:\/\/(www\.)?([^/?#]*).*$/s, "$2")}`;
      }
      return li;
    }));
  }

  function open(li, button) {
    const shown = li.querySelector("blockquote");
    if (shown) {
      shown.remove();
      button.setAttribute("aria-expanded", "false");
      return;
    }
    button.setAttribute("aria-expanded", "true");
    worker.postMessage({ type: "around", id: li.dataset.id, ref: li.dataset.ref, before: 8, after: 12 });
  }

  // The lines around a result: each speaker's name where it changes, stage directions in italic, the found line current.
  function context(li, lines) {
    const quote = document.createElement("blockquote");
    let who = null;
    for (const line of lines) {
      const speaker = line.fields.speaker;
      if (speaker && speaker !== who) quote.append(Object.assign(document.createElement("b"), { textContent: speaker }));
      if (speaker) who = speaker;
      const p = document.createElement("p");
      if (line.fields.kind === "stage") p.appendChild(document.createElement("i")).textContent = line.text;
      else p.textContent = line.text;
      p.title = line.url;
      if (line.here) p.setAttribute("aria-current", "true");
      quote.append(p);
    }
    li.append(quote);
  }

  box.addEventListener("input", find);
  box.form?.addEventListener("submit", (e) => e.preventDefault());

  // Keys: / to search from anywhere on the page that isn't a text box, then, inside the element, j and k (or the
  // arrows) to move through results, Enter to open one, o to follow its link, and Escape back to the search box.
  function move(by) {
    const all = [...results.querySelectorAll("li > button")];
    const at = all.indexOf(document.activeElement?.closest("li")?.querySelector("button"));
    const to = at + by;
    if (to < 0) return box.focus();
    if (to < all.length) {
      all[to].focus();
      all[to].scrollIntoView({ block: "nearest" });
    }
  }

  addEventListener("keydown", (e) => {
    if (e.metaKey || e.ctrlKey || e.altKey || e.defaultPrevented) return;
    const at = document.activeElement;
    const typing = at === box;
    if (e.key === "/" && !box.disabled && !at?.closest("input, textarea, select, [contenteditable]")) {
      e.preventDefault();
      box.focus();
      box.select();
    }
    if (!root.contains(at)) return;
    if (e.key === "ArrowDown" || (e.key === "j" && !typing)) {
      e.preventDefault();
      move(1);
    } else if (e.key === "ArrowUp" || (e.key === "k" && !typing)) {
      if (typing) return;
      e.preventDefault();
      move(-1);
    } else if (e.key === "o" && !typing) {
      const link = at.closest("li")?.dataset.link;
      if (link) window.open(link, "_blank", "noopener");
    } else if (e.key === "Escape" && !typing) {
      box.focus();
    }
  });

  // The credit line may name a web address or a text file shipped beside the page; make each a link.
  for (const p of root.querySelectorAll("footer p")) {
    p.innerHTML = p.innerHTML.replace(/https?:\/\/[^\s<]+[^\s<.,)]|\b[\w.-]+\.txt\b/g, (url) => `<a href="${url}">${url}</a>`);
  }

  if (!drop) return;

  // Dropped or chosen files: every text file is read and indexed in the worker.
  async function index(files) {
    const texts = [];
    for (const { path, file } of files) {
      if (!TEXT.test(path) && path.includes(".")) continue;
      if (file.size > 50e6) continue;
      texts.push({ path, text: await file.text() });
    }
    if (!texts.length) return say("No text files there. Text, Markdown, HTML and TSV work.");
    say(`Indexing ${texts.length.toLocaleString()} files…`);
    worker.postMessage({ type: "files", files: texts });
  }

  async function walk(entry, out) {
    if (entry.isFile) {
      out.push({ path: entry.fullPath.replace(/^\//, ""), file: await new Promise((ok, fail) => entry.file(ok, fail)) });
    } else if (entry.isDirectory && !entry.name.startsWith(".")) {
      const reader = entry.createReader();
      for (;;) {
        const batch = await new Promise((ok, fail) => reader.readEntries(ok, fail));
        if (!batch.length) break;
        for (const child of batch) await walk(child, out);
      }
    }
  }

  const dragging = (on) => {
    try {
      internals.states[on ? "add" : "delete"]("dragging");
    } catch {}
  };
  addEventListener("dragover", (e) => {
    e.preventDefault();
    dragging(true);
  });
  addEventListener("dragleave", (e) => {
    if (!e.relatedTarget) dragging(false);
  });
  addEventListener("drop", async (e) => {
    e.preventDefault();
    dragging(false);
    const files = [];
    for (const item of e.dataTransfer.items) {
      const entry = item.webkitGetAsEntry?.();
      if (entry) await walk(entry, files);
    }
    index(files);
  });
  for (const input of drop.querySelectorAll("input[type=file]")) {
    input.addEventListener("change", (e) => index([...e.target.files].map((file) => ({ path: file.webkitRelativePath || file.name, file }))));
  }
}
