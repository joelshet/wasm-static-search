// Runs the Rust engine off the main thread so typing never waits on a search.
import init, * as engine from "./pkg/search_wasm.js";

let ready = false;
let pending = null; // newest search not yet run; older ones are dropped
let scheduled = false;

async function fetchBytes(url) {
  const res = await fetch(url);
  if (!res.ok) return null;
  let bytes = new Uint8Array(await res.arrayBuffer());
  // Some hosts already strip the gzip layer via Content-Encoding; only inflate real gzip.
  if (bytes[0] === 0x1f && bytes[1] === 0x8b) {
    const stream = new Blob([bytes]).stream().pipeThrough(new DecompressionStream("gzip"));
    bytes = new Uint8Array(await new Response(stream).arrayBuffer());
  }
  return bytes;
}

async function start() {
  try {
    const t0 = performance.now();
    const [, bytes] = await Promise.all([
      init({ module_or_path: new URL("./pkg/search_wasm_bg.wasm", import.meta.url) }),
      fetchBytes(self.name || new URL("./index.bin", import.meta.url)),
    ]);
    if (bytes && !engine.load_index(bytes)) throw new Error("index.bin is not an index");
    ready = Boolean(bytes);
    postMessage({ type: "ready", meta: ready ? JSON.parse(engine.meta()) : null, ms: performance.now() - t0 });
    if (pending) schedule();
  } catch (e) {
    postMessage({ type: "error", message: String(e?.message || e) });
  }
}

function schedule() {
  if (scheduled || !ready) return;
  scheduled = true;
  // Let any keystrokes already queued arrive first, then search only the newest.
  setTimeout(() => {
    scheduled = false;
    const m = pending;
    pending = null;
    if (!m) return;
    const t0 = performance.now();
    const results = JSON.parse(engine.search(m.q, m.max || 20));
    postMessage({ type: "results", id: m.id, q: m.q, results, ms: performance.now() - t0 });
  }, 0);
}

onmessage = (e) => {
  const m = e.data;
  if (m.type === "search") {
    pending = m;
    schedule();
  } else if (m.type === "files") {
    const t0 = performance.now();
    const count = engine.index_files(JSON.stringify(m.files), m.by || "");
    ready = count > 0;
    postMessage({ type: "indexed", count, meta: ready ? JSON.parse(engine.meta()) : null, ms: performance.now() - t0 });
  } else if (m.type === "around") {
    postMessage({ type: "around", id: m.id, lines: JSON.parse(engine.around(m.ref, m.before, m.after)) });
  }
};

start();
