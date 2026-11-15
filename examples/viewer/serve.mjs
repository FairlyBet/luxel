// Dependency-free static server for this folder (serves .wasm with the right
// MIME type, which streaming instantiation needs).
//
//   node serve.mjs [port] [--lan]
//
// Without --lan it listens on 127.0.0.1 only. With --lan it listens on every
// interface and prints this machine's local-network addresses, so a phone on
// the same Wi-Fi can open the viewer (allow Node through the firewall if
// asked).
//
// POST /trace saves a profiling report the page sends (its Profiling panel)
// under traces/ at the repository root.
import { createServer } from "node:http";
import { mkdir, readFile, writeFile } from "node:fs/promises";
import { networkInterfaces } from "node:os";
import { extname, join, normalize } from "node:path";
import { fileURLToPath } from "node:url";

const root = fileURLToPath(new URL(".", import.meta.url));
const traces = fileURLToPath(new URL("../../traces/", import.meta.url));
const args = process.argv.slice(2);
const lan = args.includes("--lan");
const port = Number(args.find((a) => /^\d+$/.test(a)) ?? 8091);
const types = {
  ".html": "text/html; charset=utf-8",
  ".js": "text/javascript",
  ".wasm": "application/wasm",
  ".css": "text/css",
  ".json": "application/json",
};

// Reports are a few MB; anything far past that isn't one.
const MAX_UPLOAD = 64 * 1024 * 1024;

const server = createServer((req, res) => {
  handle(req, res).catch((e) => {
    // A bad request or a failed write answers 400/500; the server goes on.
    console.error(`request failed: ${e instanceof Error ? e.message : e}`);
    if (!res.headersSent) res.writeHead(e instanceof URIError ? 400 : 500);
    res.end();
  });
});

async function handle(/** @type {import("node:http").IncomingMessage} */ req, /** @type {import("node:http").ServerResponse} */ res) {
  if (req.method === "POST") {
    const url = new URL(req.url ?? "/", "http://x");
    if (url.pathname !== "/trace") {
      res.writeHead(404).end();
      return;
    }
    // From the viewer's own pages only: another site's page can't post here
    // (a cross-origin form or fetch says where it comes from).
    const origin = req.headers.origin;
    if (origin && new URL(origin).host !== req.headers.host) {
      res.writeHead(403).end();
      return;
    }
    const chunks = [];
    let size = 0;
    for await (const c of req) {
      size += c.length;
      if (size > MAX_UPLOAD) {
        res.writeHead(413).end();
        req.destroy();
        return;
      }
      chunks.push(c);
    }
    // A plain file name in traces/, nothing else.
    const name = (url.searchParams.get("name") ?? "trace").replace(/[^\w.-]+/g, "_").replace(/^\.+/, "").slice(0, 120).replace(/(\.json)?$/, ".json");
    await mkdir(traces, { recursive: true });
    const file = join(traces, name);
    await writeFile(file, Buffer.concat(chunks));
    console.log(`trace saved: ${file}`);
    res.writeHead(200, { "content-type": "text/plain" }).end(`traces/${name}`);
    return;
  }
  const path = normalize(decodeURIComponent(new URL(req.url ?? "/", "http://x").pathname));
  const file = join(root, /[\\/]$/.test(path) ? path + "index.html" : path);
  if (!file.startsWith(root)) {
    res.writeHead(403).end();
    return;
  }
  try {
    const body = await readFile(file);
    res.writeHead(200, {
      "content-type": types[extname(file)] ?? "application/octet-stream",
      "cache-control": "no-cache",
    });
    res.end(body);
  } catch {
    res.writeHead(404).end();
  }
}

server.listen(port, lan ? "0.0.0.0" : "127.0.0.1", () => {
  console.log(`http://127.0.0.1:${port}/`);
  if (lan) {
    for (const list of Object.values(networkInterfaces())) {
      for (const a of list ?? []) {
        if (a.family === "IPv4" && !a.internal) console.log(`http://${a.address}:${port}/`);
      }
    }
  }
});
