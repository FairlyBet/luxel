// Where a model's files come from: one object per `loadModel` call that every
// request of the load goes through — the file itself, its external buffers
// and its images (also those that arrive after `loadModel` resolves).
//
// - `signal` aborts every request still running (and the load, if it hasn't
//   resolved yet).
// - `request` (a `RequestInit`: headers, credentials, mode, …) goes with every
//   fetch.
// - `resolveUrl(url)` rewrites each absolute URL before it's fetched, as
//   three.js' `LoadingManager.setURLModifier`: signed CDN URLs, a proxy, or a
//   blob URL of the app's own.
// - `files` (an array of `File`s, a `FileList`, or a name → `Blob` map /
//   object) serves files a user dropped: a URI the model names is looked up
//   by its path, then by its file name.
// - `onProgress(loaded, total)`: bytes so far over all requests; `total`
//   adds up the sizes known (`Content-Length`, a blob's size), so it grows as
//   requests start and is 0 while none is known.

// The base URL of a model given as a `File` / `Blob` / bytes with `files`:
// its relative URIs resolve under it, then are looked up in `files`.
const FILES_BASE = "https://files.invalid/";

// AbortSignal.any where the browser has it (Safari 17.4, Chrome 116,
// Firefox 124), else the same by hand: aborted when any of `list` is.
function anySignal(list) {
  if (AbortSignal.any) return AbortSignal.any(list);
  const c = new AbortController();
  for (const s of list) {
    if (s.aborted) {
      c.abort(s.reason);
      break;
    }
    s.addEventListener("abort", () => c.abort(s.reason), { once: true });
  }
  return c.signal;
}

// `s` with each %XX decoded where it can be: a bare "%" (a file named
// "50%.png") stays as it is instead of failing the whole path.
function decodePath(s) {
  try {
    return decodeURIComponent(s);
  } catch {
    return s.replace(/(%[0-9A-Fa-f]{2})+/g, (run) => {
      try {
        return decodeURIComponent(run);
      } catch {
        return run;
      }
    });
  }
}

export class Fetcher {
  constructor(options) {
    const o = options ?? {};
    // The app's signal: given on its own or in `request` (one of them, or
    // both combined). Kept out of `request`, which a reload reuses.
    const { signal: inRequest, ...request } = o.request ?? {};
    const signals = [o.signal, inRequest].filter(Boolean);
    this.signal = signals.length > 1 ? anySignal(signals) : (signals[0] ?? null);
    this.request = request;
    // The load's own: stops what's still running once the load has failed.
    this.stop = new AbortController();
    this.fetchSignal = this.signal ? anySignal([this.signal, this.stop.signal]) : this.stop.signal;
    this.resolveUrl = typeof o.resolveUrl === "function" ? o.resolveUrl : null;
    this.onProgress = typeof o.onProgress === "function" ? o.onProgress : null;
    this.files = filesMap(o.files);
    this.loaded = 0;
    this.total = 0;
  }

  aborted() {
    return this.signal?.aborted === true;
  }

  // Throws the signal's reason (an `AbortError` by default) once aborted.
  throwIfAborted() {
    this.signal?.throwIfAborted();
    this.stop.signal.throwIfAborted();
  }

  abortReason() {
    return this.signal?.reason;
  }

  // The load failed: requests still running stop, and progress is no
  // longer reported.
  cancel() {
    this.onProgress = null;
    this.stop.abort();
  }

  // The main file: `{ bytes, base, name }` from a URL, a `File` / `Blob`, or
  // bytes (`ArrayBuffer`, a typed array or `DataView`).
  async main(source, baseUrl) {
    if (typeof source === "string") {
      const url = new URL(source, location.href).href;
      return { bytes: await this.bytes(url), base: url, name: source };
    }
    if (source instanceof Blob) {
      // A file from a picked directory: by its path in it (its URIs resolve
      // beside it).
      const path = source.webkitRelativePath
        ? source.webkitRelativePath.split("/").slice(1).join("/")
        : typeof source.name === "string"
          ? source.name
          : "model";
      const base = baseUrl
        ? new URL(baseUrl, location.href).href
        : this.files.size
          ? FILES_BASE + encodePath(path)
          : location.href;
      return { bytes: await this.blobBytes(source), base, name: source.name ?? "" };
    }
    let bytes;
    if (source instanceof ArrayBuffer) bytes = new Uint8Array(source);
    else if (ArrayBuffer.isView(source))
      bytes = new Uint8Array(source.buffer, source.byteOffset, source.byteLength);
    else throw new TypeError("loadModel: a URL, a File / Blob or bytes");
    const base = baseUrl
      ? new URL(baseUrl, location.href).href
      : this.files.size
        ? FILES_BASE
        : location.href;
    this.count(bytes.byteLength, bytes.byteLength);
    return { bytes, base, name: "" };
  }

  // A file's bytes (a buffer, a KTX2 image).
  async bytes(url) {
    const blob = this.lookup(url);
    if (blob) return this.blobBytes(blob);
    const response = await this.fetch(url);
    return new Uint8Array(await this.read(response));
  }

  // A file as a `Blob` (an image, for the browser's decoder): with its type
  // from the server, or `type` when it has none.
  async blob(url, type) {
    const local = this.lookup(url);
    if (local) {
      this.throwIfAborted();
      this.count(local.size, local.size);
      return local.type ? local : new Blob([local], { type });
    }
    const response = await this.fetch(url);
    const served = response.headers.get("content-type") ?? "";
    const kind = served.startsWith("image/") ? served : type;
    return new Blob([await this.read(response)], { type: kind });
  }

  // `url` after `resolveUrl`, fetched with the load's request options.
  async fetch(url) {
    this.throwIfAborted();
    const target = this.resolveUrl ? String(this.resolveUrl(url)) : url;
    const init = { ...this.request };
    init.signal = this.fetchSignal;
    const response = await fetch(target, init);
    if (!response.ok) throw new Error(`fetch "${target}" failed: HTTP ${response.status}`);
    return response;
  }

  // The body, counted into the progress as it streams. `Content-Length` is
  // a guess: compressed in transit it counts the encoded bytes while the
  // stream gives the decoded ones (and cross-origin the encoding can't be
  // read), so the total grows to what arrives, and ends exact.
  async read(response) {
    const expected = Number(response.headers.get("content-length")) || 0;
    if (!this.onProgress || !response.body) {
      const body = await response.arrayBuffer();
      this.count(body.byteLength, body.byteLength);
      return body;
    }
    this.count(0, expected);
    let counted = expected;
    const reader = response.body.getReader();
    const chunks = [];
    let length = 0;
    for (;;) {
      const { done, value } = await reader.read();
      if (done) break;
      chunks.push(value);
      length += value.byteLength;
      // A size not announced counts in the total only once it's known (at
      // the end): `total` doesn't run along with `loaded` meanwhile.
      const grow = expected > 0 ? Math.max(0, length - counted) : 0;
      counted += grow;
      this.count(value.byteLength, grow);
    }
    if (expected === 0) this.count(0, length);
    // Less arrived than announced: the total comes down to it.
    else if (counted > length) this.count(0, length - counted);
    const out = new Uint8Array(length);
    let at = 0;
    for (const c of chunks) {
      out.set(c, at);
      at += c.byteLength;
    }
    return out.buffer;
  }

  async blobBytes(blob) {
    this.throwIfAborted();
    const bytes = new Uint8Array(await blob.arrayBuffer());
    this.throwIfAborted();
    this.count(bytes.byteLength, bytes.byteLength);
    return bytes;
  }

  // The dropped file `url` names, if any: under FILES_BASE by its path, then
  // anywhere by its file name.
  lookup(url) {
    if (!this.files.size) return null;
    let path;
    try {
      path = decodePath(new URL(url).pathname.replace(/^\//, ""));
    } catch {
      return null;
    }
    const byPath = url.startsWith(FILES_BASE) ? this.files.get(path) : undefined;
    const found = byPath ?? this.files.get(path.split("/").pop());
    if (found) return found;
    if (url.startsWith(FILES_BASE)) throw new Error(`"${path}" isn't among the files given`);
    return null;
  }

  count(loaded, total) {
    this.loaded += loaded;
    this.total += total;
    if (this.onProgress && (loaded || total)) {
      try {
        this.onProgress(this.loaded, this.total);
      } catch (e) {
        console.error(e);
      }
    }
  }

  // The options again for a reload after a context loss: the same sources
  // and request options, without the signal and progress of the first load.
  forReload() {
    const f = new Fetcher({ request: this.request, resolveUrl: this.resolveUrl });
    f.files = this.files;
    return f;
  }
}

// name / path → Blob. Each `File` goes in under its name and, from a picked
// directory, its relative path too.
function filesMap(files) {
  const map = new Map();
  if (!files) return map;
  if (files instanceof Map) {
    for (const [k, v] of files) map.set(String(k), v);
    return map;
  }
  if (typeof files[Symbol.iterator] === "function" || typeof files.length === "number") {
    for (const f of Array.from(files)) {
      map.set(f.name, f);
      if (f.webkitRelativePath) {
        // Relative to the picked directory: its first segment dropped.
        map.set(f.webkitRelativePath.split("/").slice(1).join("/"), f);
      }
    }
    return map;
  }
  for (const [k, v] of Object.entries(files)) map.set(k, v);
  return map;
}

function encodePath(name) {
  return name.split("/").map(encodeURIComponent).join("/");
}

export function makeFetcher(options) {
  return new Fetcher(options);
}
