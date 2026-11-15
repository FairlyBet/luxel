# Review, phase 2 — what is missing or weak

Four read-only reviews (foundation + ECS, core, glTF, scene + JS API), each
claim checked against the code, merged and grouped into pieces of work.
Effort S / M / L; value H / M / L. Nothing here is fixed yet: pick from it.

## 1. Loading (glTF) — H, M (one piece of work)

Two reviews independently put this first; it is what an app meets first.

- **From bytes / `File`, with a URL resolver.** `load_model_with` takes only
  a URL (`gltf/src/lib.rs`, `fetch.rs`). A dropped `.gltf` with its `.bin`
  and textures can't resolve them (the viewer's `blob:` trick works for a
  `.glb` only). Wanted: `parseModel(ctx, bytes, baseUrl)` and a
  three.js-style URL modifier.
- **Abort, progress, request options.** `fetch_bytes` calls `fetch(url)`
  with no `RequestInit`: no `AbortSignal` (the viewer keeps its own
  tickets), no progress bar, no headers / credentials (signed CDN URLs).
  External buffers download one after another.
- **"Fully loaded" and errors.** `loadModel` resolves before the images;
  `whenReady` waits for shaders and the environment, not textures, so a
  product pops in black first. Image / KTX2 / ORM failures go only to the
  console. Wanted: `model.whenLoaded()` (the texture callbacks already
  exist) and `model.warnings()`.
- **Per-load options.** Texture and vertex compression are module-global
  "from now on" switches; with `LoadOptions` now in Rust, take them per
  call from JS too.
- **Lenient validation.** One validation error rejects the whole file
  (`gltf::Document::from_json`); three.js loads such files with warnings.
- `loadModel` / `loadTexture` / `whenReady` are `Promise<any>`.

## 2. Typed JS API — H, S–M

- **Branded handles** (`Entity`, `MaterialId`, `TextureId`, `ProgramId`…)
  instead of bare `number`: the scene's API takes entities in ~160 places,
  the core's `Pass.push` takes seven numbers in a row. wasm-bindgen's
  `unchecked_param_type` / `unchecked_return_type`, or a pass over the
  `.d.ts` in `build-npm.mjs`.
- **Typed promises and callbacks** (`Promise<Model>`, `Promise<Texture>`;
  `Function` callbacks typed).
- **One vocabulary**: `setToneMapping` / `setShadingModel` take strings
  while `ToneMapping` / `ShadingModel` enums are exported too.
- **`--weak-refs`** in the wasm-pack builds: wasm-bindgen objects
  (`frameStats()` every frame, `Pass`, `RenderState`…) are freed by the
  GC instead of leaking without `free()`. Handles stay manual; document it.

## 3. A viewer's building blocks in the library — H, M

What every product viewer rebuilds today from the example's code:

- **Render on demand**: `scene.needsRender()` / `invalidate()` set by the
  scene's own setters and async work (TAA settled, shaders, bakes,
  animation). The viewer stitches it from five signals.
- **Camera controls and framing**: a shipped orbit controller (a JS
  module beside the package is enough) and `frameBounds(camera, root)`.
- **Raycast and projection**: synchronous `raycast(camera, x, y)` and
  `worldToScreen` / `screenToRay` (Rust has `Ray`, `pick_bounds`,
  `ray_aabb`, not exposed). Hotspots and annotations need projection.
- **Quality presets and resolution**: `setQuality("low"|"medium"|"high")`,
  a pixel-ratio / resolution scale, adaptive resolution from GPU timings.
- **Screenshots**: `capture(w, h) → Blob` (thumbnails, "save image");
  `render(camera, pass)` today skips HDR, AA, background and post.

## 4. Configurator correctness — H, M

- **Variant materials get the default material's vertex arrays** (UV
  slots, generated tangents): a variant using another TEXCOORD set or
  adding a normal map renders wrong. Variants are the configurator
  feature.
- **Instancing / cloning a model** (sharing GPU data): today the same
  product twice means two loads and twice the memory.
- A texture used as color and as data is built once, sRGB (S).
- Morph targets: bounds from the targets' deltas (S); today morphing
  meshes aren't culled at all.

## 5. ECS foundations — H, S (do 5.1–5.3 together)

- **5.1 One tick rule for systems**: `World::insert` / `get_mut` stamp
  the next system's tick, so a system's own writes look changed to it
  next run. Add stamping at a given tick (or `SystemTicks` begin/end).
- **5.2 O(1) "changed since" per storage** (`newest_change`, a structural
  version): static frames still compare every `Parent`, scan every
  `Visible` (7 places), hash per-row ticks.
- **5.3 Removals with their tick** (`removed_since(last_run)`) instead of a
  per-frame log with a call-order contract and workarounds.
- Then (M): a **hierarchy index** (children from the transform system's
  order — `children`/`descendants` build a `HashMap` of every `Parent` per
  call) and **inherited visibility** computed once instead of an ancestor
  walk per mesh; **queries over 3+ components** with optional / without
  (hand-written ~6 times); mutable access to N storages (animation clones
  every playing clip each frame to dodge the borrow); `EntityMap<T>`.
- Docs: rows are unstable across removals; which calls panic; the two
  stamping behaviours. Not wanted: a scheduler, resources, events.

## 6. Core gaps — M

- **Shader errors with parallel compile**: `programsReady` never rejects
  and there's no info-log getter; editors, hot reload and fallbacks need
  the error text (S–M, H).
- **MSAA resolve beyond color 0**: blit and `set_resolve` read attachment
  0 only; MRT, depth, velocity or ID resolves have no route (S, H for
  G-buffers / TAA with MSAA).
- **`ctx.limits()` and `gpuInfo()` from JS** (S): e.g.
  `paramSetBlockRange` needs an alignment JS can't read.
- **Profiler**: with `time()` → `update()` → render it records nothing
  (a slot freed before it ran) — a defect, S.
- **Async consistency**: image loads take callbacks reporting failure as
  (0, 0); promises untyped; no `AbortSignal`.
- **Resource labels** (`setLabel`) in errors, traces and stats (M);
  **resource stats** (live counts, estimated GPU memory) for leak hunting
  (S).
- **VaoBuilder**: no per-instance attributes, no dynamic usage, its
  buffers unreachable (S–M).
- **Raw GL interop** (`rawGl`, `textureRaw`, import an external
  framebuffer): needed for WebXR and sharing with other engines (S–M; H
  if XR is planned).
- Error type: JS can't tell a lost context from a compile error (S).

## 7. Scene API polish — M

- Getters for every setter (`exposure`, `toneMapping`, `environmentRotation`,
  `samples`, …), `children(e)`; one getter style (`isHdr` vs
  `ambientOcclusion`); setters that clamp say so (return the value).
- Animation: seek (`setAnimationTime`), crossfade / weights, several
  clips per root, finished / loop events; `KHR_animation_pointer`,
  `KHR_node_visibility` (S, easy with visibility).
- Background colour / gradient / transparent; environment from KTX2 or
  cube maps, EXR; fog.
- `whenReady` covering model reloads after a context loss; scene-level
  lost / restored events.
- JS API docs: a package README (loop, `whenReady`, resize, destroy,
  ownership of `addModel`'s `Model`, what `cloneMaterial` shares).

## 8. Later — L

Post-processing hooks (bloom is in TODO), material extension hooks
(`onBeforeCompile`-like), layers / render order, multiple viewports and
offscreen full-quality renders, zero-copy JS views of storages, quantized
vertex attributes kept on the GPU, `KHR_xmp_json_ld`, pbrSpecularGlossiness,
multiple glTF scenes, WebXR.

## Suggested order

1. **Typed JS API (2)** — small, prevents the commonest mistakes.
2. **Loading (1)** — the first thing an app meets.
3. **ECS 5.1–5.3** — small, the base for hierarchy / visibility / queries.
4. **Viewer building blocks (3)** — render on demand, orbit, raycast.
5. **Configurator correctness (4)**, then **core gaps (6)** by need.
