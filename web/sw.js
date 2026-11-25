// Offline after the first visit: everything the page needs is cached up front.
const VERSION = "dev";
const FILES = ["./"];
// Folders saved as the page fetches from them, rather than up front, for files a visitor may never need.
const LATER = [];
// Caches belong to the whole origin, so the name carries this page's path: two searches on one site keep their own.
const SCOPE = new URL("./", self.location).pathname;
const CACHE = `search-${SCOPE}-${VERSION}`;

// `search-cli bundle` stamps VERSION with a content hash and lists FILES; unstamped means local development.
const DEV = VERSION === "dev";

self.addEventListener("install", (e) => {
  if (DEV) return self.skipWaiting();
  e.waitUntil(saveFiles().then(() => self.skipWaiting()));
});

// Each file is saved as it arrives, so an install cut short carries on from where it stopped next time.
async function saveFiles() {
  const cache = await caches.open(CACHE);
  const saved = new Set((await cache.keys()).map((r) => r.url));
  await Promise.all(FILES.map(async (f) => {
    // "no-cache" asks the server whether the browser's copy is current and downloads only when it isn't.
    const req = new Request(f, { cache: "no-cache" });
    if (saved.has(req.url)) return;
    const res = await fetch(req);
    if (!res.ok) throw new Error(`${f}: ${res.status}`);
    await cache.put(req, res);
  }));
}

// Only this page's old caches go.
const stale = (k) => k !== CACHE && k.startsWith(`search-${SCOPE}-`);

self.addEventListener("activate", (e) => {
  e.waitUntil(
    caches.keys()
      .then((keys) => Promise.all(keys.filter(stale).map((k) => caches.delete(k))))
      .then(() => self.clients.claim()),
  );
});

self.addEventListener("fetch", (e) => {
  const url = new URL(e.request.url);
  if (DEV || e.request.method !== "GET" || url.origin !== location.origin) return;
  // Pages carry the query in ?q=..., so match them against the cached shell.
  const page = e.request.mode === "navigate";
  const later = LATER.some((f) => url.pathname.startsWith(SCOPE + f));
  e.respondWith(caches.match(e.request, { ignoreSearch: page }).then((hit) => hit || fetch(e.request).then(async (res) => {
    if (later && res.ok) await (await caches.open(CACHE)).put(e.request, res.clone());
    return res;
  })));
});
