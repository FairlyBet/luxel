// Gives the `.d.ts` wasm-bindgen writes branded handle types.
//
// Every handle (`Entity`, `TextureId`, `ProgramId`, …) crosses to JS as a
// plain `number`, so wasm-bindgen types it `number`: a texture fits where an
// entity is wanted, and `Pass.push`'s seven numbers in a row take any order.
// This reads the Rust sources for the exported functions, methods and fields
// whose parameters or results are handles, and rewrites those `number`s in
// the `.d.ts` as
//
//   export type TextureId = number & { readonly __brand: "TextureId" };
//
// Only types change: at run time a handle is still a number. Anything the
// script expects but doesn't find in the `.d.ts` stops the build, so the
// typings can't drift from the code silently.
//
//   node scripts/brand-handles.mjs <file.d.ts>   (build-npm.mjs runs it)
import { readdirSync, readFileSync, statSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

const root = fileURLToPath(new URL("..", import.meta.url));
const camel = (s) => s.replace(/_([a-z0-9])/g, (_, c) => c.toUpperCase());

function rustFiles(dir, out = []) {
  for (const name of readdirSync(dir)) {
    const p = join(dir, name);
    if (statSync(p).isDirectory()) rustFiles(p, out);
    else if (name.endsWith(".rs")) out.push(p);
  }
  return out;
}

// Comments and string contents blanked (same length), so braces and parens
// inside them don't count. Strings keep their quotes; attribute values are
// read from the original text at the same offsets.
function blank(src) {
  let out = "";
  let i = 0;
  while (i < src.length) {
    const c = src[i];
    const n = src[i + 1];
    if (c === "/" && n === "/") {
      const end = src.indexOf("\n", i);
      const stop = end < 0 ? src.length : end;
      out += " ".repeat(stop - i);
      i = stop;
    } else if (c === "/" && n === "*") {
      let depth = 1;
      let j = i + 2;
      while (j < src.length && depth > 0) {
        if (src[j] === "/" && src[j + 1] === "*") (depth++, (j += 2));
        else if (src[j] === "*" && src[j + 1] === "/") (depth--, (j += 2));
        else j++;
      }
      out += src.slice(i, j).replace(/[^\n]/g, " ");
      i = j;
    } else if (c === "r" && /^r#*"/.test(src.slice(i, i + 10)) && !/\w/.test(src[i - 1] ?? "")) {
      const hashes = src.slice(i + 1).match(/^#*/)[0];
      const close = '"' + hashes;
      const start = i + 2 + hashes.length;
      const end = src.indexOf(close, start);
      out += src.slice(i, start) + src.slice(start, end).replace(/[^\n]/g, " ") + close;
      i = end + close.length;
    } else if (c === '"') {
      let j = i + 1;
      while (j < src.length && src[j] !== '"') j += src[j] === "\\" ? 2 : 1;
      out += '"' + src.slice(i + 1, j).replace(/[^\n]/g, " ") + '"';
      i = j + 1;
    } else if (c === "'" && /^'(\\.|[^\\'])'/.test(src.slice(i, i + 4))) {
      // A char literal (not a lifetime).
      const len = src.slice(i, i + 4).match(/^'(\\.|[^\\'])'/)[0].length;
      out += "'" + " ".repeat(len - 2) + "'";
      i += len;
    } else {
      out += c;
      i++;
    }
  }
  return out;
}

// The index just past the bracket matching the one at `open` (round,
// square or curly; angle brackets aren't tracked).
function matching(s, open) {
  const pairs = { "(": ")", "{": "}", "[": "]" };
  const stack = [pairs[s[open]]];
  let i = open + 1;
  while (i < s.length && stack.length) {
    const c = s[i];
    if (c === stack[stack.length - 1]) stack.pop();
    else if (c in pairs) stack.push(pairs[c]);
    i++;
  }
  return i;
}

// Splits on top-level commas.
function splitTop(s) {
  const parts = [];
  let depth = 0;
  let start = 0;
  for (let i = 0; i < s.length; i++) {
    const c = s[i];
    if ("([{<".includes(c)) depth++;
    else if (")]}".includes(c) || (c === ">" && !"-=".includes(s[i - 1]))) depth--;
    else if (c === "," && depth === 0) {
      parts.push(s.slice(start, i));
      start = i + 1;
    }
  }
  parts.push(s.slice(start));
  return parts.map((p) => p.trim()).filter(Boolean);
}

// The attributes written just before `at` (`#[...]`, possibly several),
// read from the original text.
function attrsBefore(clean, orig, at) {
  const attrs = [];
  let i = at;
  for (;;) {
    let j = i - 1;
    while (j >= 0 && /\s/.test(clean[j])) j--;
    if (clean[j] !== "]") break;
    // Walk back to its `#[`.
    let depth = 0;
    let k = j;
    for (; k >= 0; k--) {
      if (clean[k] === "]") depth++;
      else if (clean[k] === "[") {
        depth--;
        if (depth === 0) break;
      }
    }
    if (clean[k - 1] !== "#") break;
    attrs.unshift(orig.slice(k - 1, j + 1));
    i = k - 1;
  }
  return attrs;
}

const handleNames = new Set();
const files = [];
for (const pkg of readdirSync(join(root, "packages"))) {
  const src = join(root, "packages", pkg, "src");
  try {
    statSync(src);
  } catch {
    continue;
  }
  for (const f of rustFiles(src)) files.push(f);
}
for (const f of files) {
  const s = blank(readFileSync(f, "utf8"));
  for (const m of s.matchAll(/typed_handle!\s*\(\s*(?:#\[[^\]]*\]\s*)*pub\s+struct\s+(\w+)/g)) {
    handleNames.add(m[1]);
  }
}
// Doc comments are blanked, so the declaration matches past them.

// A Rust type → the brand it carries, `Brand | undefined` for an option, or
// null when it isn't a handle.
function brandOf(ty) {
  const t = ty.replace(/\s+/g, "");
  const last = (p) => p.split("::").pop();
  const opt = t.match(/^Option<(.+)>$/);
  if (opt && handleNames.has(last(opt[1]))) return { brand: last(opt[1]), optional: true };
  if (handleNames.has(last(t))) return { brand: last(t), optional: false };
  return null;
}

const jsName = (attrs, fallback) => {
  for (const a of attrs) {
    const m = a.match(/js_name\s*=\s*"?([\w$]+)"?/);
    if (m) return m[1];
  }
  return fallback;
};
const isExport = (attrs) => attrs.some((a) => /^#\[wasm_bindgen\b/.test(a));
const has = (attrs, word) => attrs.some((a) => new RegExp(`^#\\[wasm_bindgen\\(.*\\b${word}\\b`).test(a));

// What to change: class (or "" for free functions) → member name → {params,
// result} with brands, or fields.
const plan = new Map();
const member = (cls, name) => {
  if (!plan.has(cls)) plan.set(cls, new Map());
  const c = plan.get(cls);
  if (!c.has(name)) c.set(name, []);
  return c.get(name);
};

function readFn(clean, orig, at, cls) {
  // `at` points at `fn`.
  const head = clean.slice(at).match(/^fn\s+(\w+)\s*(<[^(]*>)?\s*\(/);
  // Inside a macro (`fn $name`): not read.
  if (!head) return null;
  const name = head[1];
  const open = at + head[0].length - 1;
  const close = matching(clean, open);
  const params = [];
  for (const p of splitTop(clean.slice(open + 1, close - 1))) {
    const start = open + 1 + clean.slice(open + 1, close - 1).indexOf(p);
    const pOrig = orig.slice(start, start + p.length);
    const unchecked = /unchecked_param_type/.test(pOrig);
    const body = p.replace(/#\[[^\]]*\]\s*/g, "");
    if (/^&?\s*(mut\s+)?self$/.test(body) || /^(mut\s+)?self\s*:/.test(body)) continue;
    const colon = body.indexOf(":");
    const pname = body.slice(0, colon).replace(/^mut\s+/, "").trim();
    params.push({ name: pname, brand: unchecked ? null : brandOf(body.slice(colon + 1)) });
  }
  const rest = clean.slice(close).match(/^\s*(->\s*([^{;]*?))?\s*(where[^{]*)?[{;]/);
  let ret = rest && rest[2] ? rest[2].trim() : "";
  const res = ret.replace(/\s+/g, "").match(/^Result<(.+),[^,]+>$/);
  if (res) ret = res[1];
  const attrs = attrsBefore(clean, orig, fnStart(clean, at));
  const result = has(attrs, "unchecked_return_type") ? null : brandOf(ret);
  return { name, attrs, params, result };
}

// The start of `pub (async) (unsafe) fn` before `fn`.
function fnStart(clean, at) {
  const before = clean.slice(0, at);
  const m = before.match(/(pub(\([^)]*\))?\s+)?((async|unsafe|const|extern\s+"\w*")\s+)*$/);
  return at - m[0].length;
}

for (const f of files) {
  const orig = readFileSync(f, "utf8");
  const clean = blank(orig);
  // Exported impl blocks.
  for (const m of clean.matchAll(/\bimpl\s+(\w+)\s*\{/g)) {
    const attrs = attrsBefore(clean, orig, m.index);
    if (!isExport(attrs)) continue;
    const cls = jsName(attrs.filter((a) => /js_class/.test(a)).map((a) => a.replace("js_class", "js_name")), m[1]);
    const open = m.index + m[0].length - 1;
    const end = matching(clean, open);
    const body = clean.slice(open, end);
    // Top-level fns of the block only.
    let depth = 0;
    for (let i = 0; i < body.length; i++) {
      const c = body[i];
      if (c === "{") depth++;
      else if (c === "}") depth--;
      else if (depth === 1 && /^fn\s/.test(body.slice(i, i + 3)) && !/\w/.test(body[i - 1])) {
        const at = open + i;
        const fnAttrs = attrsBefore(clean, orig, fnStart(clean, at));
        // Plain `pub` only: wasm-bindgen doesn't export `pub(crate)`.
        if (!/^pub\s/.test(clean.slice(fnStart(clean, at), at))) continue;
        if (fnAttrs.some((a) => /skip/.test(a))) continue;
        const fn = readFn(clean, orig, at, cls);
        if (!fn) continue;
        if (!fn.params.some((p) => p.brand) && !fn.result) continue;
        let name = jsName(fnAttrs, fn.name);
        const kind = has(fnAttrs, "constructor")
          ? "constructor"
          : has(fnAttrs, "getter")
            ? "getter"
            : has(fnAttrs, "setter")
              ? "setter"
              : "method";
        if (kind === "constructor") name = "constructor";
        // A setter's `js_name` is the property's; without one, wasm-bindgen
        // drops the Rust name's `set_`.
        if (kind === "setter" && name === fn.name) name = camel(name.replace(/^set_/, ""));
        member(cls, name).push({ kind, params: fn.params, result: fn.result, where: f });
      }
    }
  }
  // Exported free functions and struct fields.
  for (const m of clean.matchAll(/\bpub\s+(async\s+)?fn\s+(\w+)/g)) {
    const at = m.index;
    const attrs = attrsBefore(clean, orig, at);
    if (!isExport(attrs)) continue;
    // Inside an exported impl? Those were read above.
    const before = clean.slice(0, at);
    const lastImpl = before.lastIndexOf("impl ");
    if (lastImpl >= 0 && matching(clean, clean.indexOf("{", lastImpl)) > at) continue;
    const fnAt = at + m[0].indexOf("fn");
    const fn = readFn(clean, orig, fnAt, "");
    if (!fn) continue;
    if (!fn.params.some((p) => p.brand) && !fn.result) continue;
    member("", jsName(attrs, fn.name)).push({ kind: "function", params: fn.params, result: fn.result, where: f });
  }
  for (const m of clean.matchAll(/\bpub\s+struct\s+(\w+)\s*\{/g)) {
    const attrs = attrsBefore(clean, orig, m.index);
    if (!isExport(attrs)) continue;
    const cls = jsName(attrs, m[1]);
    const open = m.index + m[0].length - 1;
    const end = matching(clean, open);
    for (const field of splitTop(clean.slice(open + 1, end - 1))) {
      const start = open + 1 + clean.slice(open + 1, end - 1).indexOf(field);
      const fOrig = orig.slice(start, start + field.length);
      if (/skip/.test(fOrig)) continue;
      const fm = field.replace(/#\[[^\]]*\]\s*/g, "").match(/^pub\s+(\w+)\s*:\s*(.+)$/s);
      if (!fm) continue;
      const brand = brandOf(fm[2]);
      if (!brand) continue;
      const fieldAttrs = [...fOrig.matchAll(/#\[[^\]]*\]/g)].map((a) => a[0]);
      member(cls, jsName(fieldAttrs, fm[1])).push({ kind: "field", result: brand, where: f });
    }
  }
}

const ts = (b) => (b.optional ? `${b.brand} | undefined` : b.brand);

export function brandHandles(dts) {
  const lines = dts.split("\n");
  const missing = [];
  // Class bodies: name → [first line, last line].
  const classes = new Map();
  for (let i = 0; i < lines.length; i++) {
    const m = lines[i].match(/^export class (\w+)/);
    if (!m) continue;
    let j = i;
    while (lines[j] !== "}") j++;
    classes.set(m[1], [i, j]);
  }
  const used = new Set();
  for (const [cls, members] of plan) {
    const range = cls === "" ? [0, lines.length - 1] : classes.get(cls);
    if (!range) {
      missing.push(`class ${cls}`);
      continue;
    }
    for (const [name, entries] of members) {
      for (const e of entries) {
        const rewrite = (re, fix) => {
          for (let i = range[0]; i <= range[1]; i++) {
            if (!re.test(lines[i])) continue;
            lines[i] = fix(lines[i]);
            return true;
          }
          return false;
        };
        let ok;
        if (e.kind === "field" || e.kind === "getter" || e.kind === "setter") {
          const b = e.kind === "setter" ? e.params.find((p) => p.brand)?.brand : e.result;
          if (!b) continue;
          used.add(b.brand);
          const get = new RegExp(`^\\s*(readonly\\s+)?get ${name}\\(\\): number( \\| undefined)?;`);
          const set = new RegExp(`^\\s*set ${name}\\(value: number( \\| null)?( \\| undefined)?\\);`);
          const field = new RegExp(`^\\s*(readonly\\s+)?${name}: number( \\| undefined)?;`);
          let got = false;
          if (e.kind !== "setter") {
            got = rewrite(get, (l) => l.replace(/\): number( \| undefined)?;/, `): ${ts(b)};`)) || got;
            got = rewrite(field, (l) => l.replace(/: number( \| undefined)?;/, `: ${ts(b)};`)) || got;
          }
          if (e.kind !== "getter") {
            got = rewrite(set, (l) => l.replace(/value: number( \| null)?( \| undefined)?\)/, `value: ${ts(b)})`)) || got;
          }
          ok = got;
        } else {
          const prefix =
            e.kind === "function"
              ? `^export function ${name}\\(`
              : e.kind === "constructor"
                ? `^\\s*constructor\\(`
                : `^\\s*(static\\s+)?${name}\\(`;
          ok = rewrite(new RegExp(prefix), (line) => {
            const open = line.indexOf("(");
            const close = matching(line, open);
            let params = splitTop(line.slice(open + 1, close - 1));
            params = params.map((p) => {
              const pname = p.slice(0, p.indexOf(":")).replace("?", "").trim();
              const rust = e.params.find((q) => q.name === pname || camel(q.name) === pname);
              if (!rust || !rust.brand) return p;
              used.add(rust.brand.brand);
              const opt = p.includes("?:") ? "?" : "";
              if (!/:\s*number(\s*\|\s*null)?(\s*\|\s*undefined)?$/.test(p)) {
                missing.push(`${cls}.${name}(${pname}) is \`${p}\`, not a number`);
                return p;
              }
              return `${pname}${opt}: ${ts(rust.brand)}`;
            });
            let tail = line.slice(close - 1);
            if (e.result) {
              used.add(e.result.brand);
              const branded = tail.replace(/^\): number( \| undefined)?;/, `): ${ts(e.result)};`);
              if (branded === tail) missing.push(`${cls}.${name}'s result is \`${tail}\`, not a number`);
              tail = branded;
            }
            return line.slice(0, open + 1) + params.join(", ") + tail;
          });
        }
        if (!ok) missing.push(`${cls ? cls + "." : ""}${name} (${e.where.slice(root.length)})`);
      }
    }
  }
  if (missing.length) {
    throw new Error("brand-handles: not found in the .d.ts:\n  " + missing.join("\n  "));
  }
  // Trailing `T | undefined` parameters (options objects) may be left out:
  // in JS a missing argument is `undefined` anyway.
  for (let i = 0; i < lines.length; i++) {
    const m = lines[i].match(/^(\s*(?:export function |static )?[\w$]+)\((.*)\)(: .*;)$/);
    if (!m || !m[2].includes("| undefined")) continue;
    const params = splitTop(m[2]);
    for (let j = params.length - 1; j >= 0; j--) {
      const p = params[j].match(/^([\w$]+)(\??): (.*) \| undefined$/);
      if (!p) break;
      params[j] = `${p[1]}?: ${p[3]}`;
    }
    lines[i] = `${m[1]}(${params.join(", ")})${m[3]}`;
  }
  // The crates' own plain aliases (`export type TextureId = number;`, so
  // the unbranded typings stand alone) give way to the brands.
  for (let i = lines.length - 1; i >= 0; i--) {
    const m = lines[i].match(/^export type (\w+) = number;$/);
    if (!m || !handleNames.has(m[1])) continue;
    const doc = i > 0 && /^\/\*\*.*\*\/$/.test(lines[i - 1]) ? 1 : 0;
    lines.splice(i - doc, 1 + doc);
    i -= doc;
  }
  const decls = [...handleNames]
    .sort()
    .map(
      (b) =>
        `/** A handle (a plain number at run time); see the Rust \`${b}\`. */\n` +
        `export type ${b} = number & { readonly __brand: "${b}" };`,
    )
    .join("\n");
  // After the leading comments.
  const first = lines.findIndex((l) => l.startsWith("export "));
  lines.splice(first, 0, "// Handle brands (scripts/brand-handles.mjs).", decls, "");
  return lines.join("\n");
}


if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1]) {
  const path = process.argv[2];
  writeFileSync(path, brandHandles(readFileSync(path, "utf8")));
  console.log(`branded ${path}`);
}
