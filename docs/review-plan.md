# Review plan

Every crate is covered: `rswebgl-foundation`, `rswebgl-ecs`, `rswebgl` (the core),
`rswebgl-gltf`, `rswebgl-scene`, and the viewer example.

Two phases. **Phase 1** finds and fixes bugs; **phase 2**, after it, looks for
what is missing or weak (API ergonomics, gaps against three.js / Bevy /
Filament, rough edges). Nothing is reviewed "all at once": the code is cut
into chunks of about 2–4k lines, one subsystem each, so every file is read
by someone with its neighbours in view and nothing falls between chunks.

## How a chunk is reviewed

- One read-only agent per chunk, given only its files (and what they call
  into, to read as needed), run a few at a time.
- It reports findings, not fixes: file:line, what goes wrong, a concrete
  scenario (inputs / state → wrong result, crash, leak), and how sure it is.
- I check every finding against the code myself (agents' claims are not
  taken as is), drop the false ones, and fix the real ones with a test where
  a test can show it. One commit per fix or per related group.
- Chunks are done in dependency order: a bug in the core shows up as odd
  behaviour in the scene, so the core goes first.

## Phase 1 — bugs

What every chunk is checked for: wrong results (math, std140 layouts, depth
conventions incl. reverse-Z, sRGB / linear, premultiplied alpha), panics in
wasm (unwrap, indexing, overflow, NaN into GL), GL state leaks between
passes, resource leaks (create without destroy, on errors and on context
loss), stale handles, off-by-one in sizes / mips / layers, caches that
don't invalidate, async races (programs linking, images loading,
compression finishing), what happens on context loss and restore.

| # | Chunk | Files | ~lines |
|---|-------|-------|--------|
| 1 | Foundation and ECS | `rswebgl-foundation/src/*`, `rswebgl-ecs/src/*` | 2.2k |
| 2 | Core: textures and targets | `texture.rs`, `sampler.rs`, `framebuffer.rs`, `renderbuffer.rs`, `pixel_unpack.rs`, `float_targets.rs`, `compressed_format.rs`, `limits.rs`, `extension.rs` | 4.3k |
| 3 | Core: the draw path | `renderer.rs`, `pass.rs`, `render_state.rs`, `draw.rs`, `depth_range.rs`, `buffer.rs` | 2.8k |
| 4 | Core: programs and parameters | `program.rs`, `shader_source.rs`, `param.rs`, `uniform_buffer.rs`, `vao.rs`, `vao_builder.rs`, `context.rs`, `context_loss.rs`, `ids.rs`, `undo.rs`, `error.rs` | 3.8k |
| 5 | Core: extras | `readback.rs`, `copy.rs`, `query.rs`, `profiler.rs`, `trace.rs`, `poller.rs`, `feedback.rs`, `texture_packer.rs`, `console.rs` | 2.9k |
| 6 | Core: texture compression | `texture_compressor.rs`, `compress/*` | 1.6k + shaders |
| 7 | glTF: geometry | `lib.rs`, `buffers.rs`, `accessor.rs`, `geometry.rs`, `vertex_format.rs`, `meshopt.rs`, `draco.rs`, `fetch.rs`, `decoders.js` | 3.2k |
| 8 | glTF: materials and the rest | `material.rs`, `textures.rs`, `ktx2.rs`, `image_formats.rs`, `compress.rs`, `pack.rs`, `scene.rs`, `animation.rs` | 2.9k |
| 9 | Scene: core | `render.rs`, `instances.rs`, `culling.rs`, `transform.rs`, `hierarchy.rs`, `camera.rs`, `model.rs`, `pick.rs`, `gpu_pick.rs`, `motion.rs`, `animation.rs` | 3.6k |
| 10 | Scene: lights and shadows | `lighting/*`, `shaders/lib/lights.glsl`, `shadows.glsl`, `ground*` | 2.9k |
| 11 | Scene: output and post | `output.rs`, `direct.rs`, `hdr.rs`, `environment.rs`, `brdf_lut.rs`, `occlusion.rs`, their shaders (TAA, present, FXAA, environment bake, GTAO) | 3.5k |
| 12 | Scene: shading | `pbr.rs`, `shader.rs`, `shaders/pbr.*`, `shaders/lib/*` (surface, brdf, OpenPBR, sheen, iridescence…) | 0.6k + 3k GLSL |
| 13 | Scene: the JS API | `js/*.rs` (state kept in sync, options validated, restore after context loss) | 3.7k |
| 14 | The viewer | `examples/viewer/*.js`, `index.html` | 1.5k |

Order: 1 → 2–6 (core, in parallel) → 7–8 (glTF) → 9–13 (scene) → 14.
Fixes land between the steps, so later chunks are read on fixed code.

## Phase 2 — what is missing or weak

After phase 1, by crate rather than by file (the question is the shape of
the API, not a line):

| # | Area | Questions |
|---|------|-----------|
| A | Foundation and ECS | Handles, pools, layouts; queries, change detection, removals, despawn, systems — what a scene needs that it hand-rolls today. |
| B | Scene (Rust and JS API) | Naming and consistency of the JS API, options vs setters, what three.js / Babylon users would look for and not find, error reporting, TypeScript types. |
| C | glTF | Extensions and corners of the spec not handled, loader options, errors on bad files, memory after load. |
| D | Core | API consistency (Result vs no-op + warn), missing GL features, what a user of the core alone would trip on. |

Phase 2's output is a list, sorted by value, for us to pick from — not
fixes straight away.
