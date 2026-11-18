// Builds the npm package `@fairlybet/luxel`: renderer, glTF loader and scene in one
// WebAssembly module (the luxel crate), as an ES module that runs in
// browsers as is and in bundlers that resolve `new URL(…, import.meta.url)`
// (Vite, webpack 5, Parcel; with esbuild, import the .wasm and pass its URL
// to init — see npm/README.md).
//
//   node scripts/build-npm.mjs          release build into dist/npm
//   node scripts/build-npm.mjs --dev    unoptimized, with debug info
//
// Then try it with `npm pack dist/npm` or `examples/npm-consumer`; publish
// with `npm publish dist/npm`.
import { execFileSync } from "node:child_process";
import { copyFileSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { brandHandles } from "./brand-handles.mjs";

const root = fileURLToPath(new URL("..", import.meta.url));
const out = join(root, "dist", "npm");
const dev = process.argv.includes("--dev");

execFileSync(
  "wasm-pack",
  [
    "build",
    "packages/luxel",
    "--target", "web",
    dev ? "--dev" : "--release",
    "--out-dir", "../../dist/npm",
    "--out-name", "luxel",
    // wasm-bindgen objects (`Pass`, `FrameStats`, `RenderState`, …) are
    // freed by the garbage collector when JS drops them; `free()` still
    // frees one at once. Handles (plain numbers) stay manual.
    "--weak-refs",
  ],
  { cwd: root, stdio: "inherit" },
);

// Handles typed by kind (`Entity`, `TextureId`, …) instead of `number`.
const dtsPath = join(out, "luxel.d.ts");
writeFileSync(dtsPath, brandHandles(readFileSync(dtsPath, "utf8")));

// wasm-pack writes a package.json from Cargo.toml; make it a proper ES
// package with an exports map.
const pkgPath = join(out, "package.json");
const generated = JSON.parse(readFileSync(pkgPath, "utf8"));
const pkg = {
  name: "@fairlybet/luxel",
  version: generated.version,
  description:
    "WebGL2 renderer in Rust/WebAssembly: a scene graph with glTF 2.0, OpenPBR shading, " +
    "image-based light, HDR output, skinning and animation — and the low-level API under it",
  license: "MIT",
  repository: { type: "git", url: "git+https://github.com/FairlyBet/rswebgl.git" },
  keywords: ["webgl", "webgl2", "wasm", "rust", "gltf", "pbr", "openpbr", "3d", "renderer"],
  type: "module",
  main: "./luxel.js",
  module: "./luxel.js",
  types: "./luxel.d.ts",
  exports: {
    ".": { types: "./luxel.d.ts", default: "./luxel.js" },
    "./orbit": { types: "./orbit.d.ts", default: "./orbit.js" },
    "./luxel_bg.wasm": "./luxel_bg.wasm",
    "./package.json": "./package.json",
  },
  files: [
    "luxel.js",
    "luxel.d.ts",
    "luxel_bg.wasm",
    "luxel_bg.wasm.d.ts",
    "orbit.js",
    "orbit.d.ts",
    "snippets/",
    "README.md",
    "LICENSE",
    "THIRD_PARTY.md",
  ],
  // The module instantiates nothing until `init()`; snippets carry JS the
  // wasm imports.
  sideEffects: ["./snippets/*"],
};
writeFileSync(pkgPath, JSON.stringify(pkg, null, 2) + "\n");
copyFileSync(join(root, "npm", "README.md"), join(out, "README.md"));
// The orbit controller, beside the module (`@fairlybet/luxel/orbit`).
for (const f of ["orbit.js", "orbit.d.ts"]) copyFileSync(join(root, "npm", f), join(out, f));
copyFileSync(join(root, "LICENSE"), join(out, "LICENSE"));
copyFileSync(join(root, "THIRD_PARTY.md"), join(out, "THIRD_PARTY.md"));
console.log(`\n${pkg.name}@${pkg.version} → ${out}`);
