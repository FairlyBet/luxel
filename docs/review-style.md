# Style, cleanliness and architecture review (2026-11-18)

After phase 2: what reads wrong, is duplicated, or sits in the wrong place —
not bugs (a few turned up and are listed first), not features. Found by one
read-only agent per chunk; each item checked against the code before it is
changed. Status: [x] done, [ ] to do, [-] dropped (why).

## Bugs found on the way

- [x] Error strings with a literal `\n` and a run of spaces (a broken `\`
  continuation): pass.rs, render_state.rs ×2, readback.rs ×2,
  renderbuffer.rs, texture.rs; program.rs joins errors over three lines.
- [-] GL objects leak when a pool is full: the raw object is made before the
  pool insert and dropped without `delete_*` (program + shaders, UBO, query,
  buffer, texture, …). With `impl From<PoolFull> for Error`. — dropped: a pool holds ~1M live objects and each slot is reused 4095 times; out of reach. The repeated error mapping is a style item below.
- [-] `render_state` returns `StateId::NULL` on a full pool instead of an
  error (later draws report a "stale render state"). — dropped: the same unreachable pool limit.
- [x] `set_reverse_z` returns `!on` without the extension (true for
  `setReverseZ(false)` on an unsupported device) — or the doc is wrong. — the code is right (it reports whether the request holds); the doc said otherwise, fixed.
- [x] scene render.rs `set_peel` writes `u_shading` into
  `lighting.last_mut()`, which is now `u_fill_b`. — the cache is now named fields.
- [x] output.rs: an incomplete grab framebuffer leaks its new depth
  renderbuffer every frame.
- [ ] Leaks on error part way: `HdrOutput::new`, `AutoExposure::new`,
  `set_fxaa`, `DirectTarget::new`, `SceneRenderer::new`'s lights (no Undo).
- [x] on_demand.rs: `needsRender` reads the raw `auto_exposure` (stays true
  with auto exposure set but HDR off). — and `taa && hdr_wanted` by hand ×3 now `taa()`; capture's 30 is `TAA_SETTLE`.
- [-] options `numbers()` trusts `length` (a huge length loops). — dropped: collecting stops at the first missing element.
- [x] `Occlusion::new` failing turns off AO only; contact shadows retry every
  frame.

## Docs and the viewer

- [ ] npm README: broken KTX2 transcoder URL; setQuality "contact shadows";
  getter list; no shadows/effects section; FXAA missing.
- [ ] README: point lights cast shadows too; AO, contact, ground shadow.
- [ ] TODO.md: contact shadows of the "brightest directional"; instantiate
  exists; KHR_node_visibility and gpuInfo done; dequantized attributes;
  branded handles; "Planned 7".
- [ ] examples/js-scene README: `render(camera, pass)` → `renderFrame`.
- [ ] docs/review-plan.md "Carried into phase 2" mostly fixed;
  review-phase2.md "Nothing here is fixed yet", getter style done, quantized
  attributes done; PROGRESS.md and docs/design status lines stale.
- [ ] viewer: redundant `invalidate()` after setters; `coarse` ×4;
  profiling.js hand-rolls gpuInfo, leaves `setSamples(0)`, extra
  whenReady; stale MSAA comment; HDR checkbox not synced; orbit hand-rolled
  beside `rswebgl/orbit`.

## Foundation, ECS, glTF

- [x] material.rs `translate` repeats `default_material`'s ~45 defaults →
  `..default_material()`; `doc_maps` once.
- [ ] Map slots by bare index (`maps[0..4]`, `maps()[2]`, `map == 0`,
  `.skip(5)`) → named slot constants.
- [ ] Stale comments: foundation "(later) its ECS", layouts "for ECS
  components"; material "v1", "their textures aren't read yet"; geometry
  locations up to 7 (8, 9 exist), "readers dequantize" (they don't);
  `attributes()` omits 512; the hand-written uniform list; misplaced
  comments in lib.rs `load`; draco `decode` returns an unused bool;
  pack.rs "(as before)".
- [ ] bufferView slicing written 4 times; the meshopt extension names twice.
- [ ] JS error → string three times (fetch, draco, ktx2).
- [ ] The compress-kind merge rule twice (compress.rs, pack.rs).
- [ ] Warnings only to the console, not `Model::warnings()` (primitive
  skipped, instancing fallback, packer unavailable); 29 hand-prefixed
  console calls.
- [ ] decoders.js externs live in ktx2.rs (draco imports them from there);
  `MAPS` / `material_maps` aliases; material-group building in lib.rs;
  lib.rs mixes types, request parsing and the pipeline.
- [ ] Duplicated cleanup (`Model::destroy` vs `load`'s failure path), skin-bit
  masking ×2, `per_key` ×2, the buffer reader closure ×5.
- [ ] `coat_anisotropy`, `subsurface_weight` lack `getter` (methods in JS).
- [ ] Unused pub: `Fetcher`/`Loading` re-export, `LoadRequest::with_options`;
  foundation `column` used only by its tests, its `Scalar` clashes with
  `layout::Scalar`.
- [ ] ECS `Mut`: two impl blocks, `stamp` re-implements `raise`.
- [ ] Raw GL type numbers in draco.rs; `PROPERTIES.len()` mirrored by 40.

## Core: context, renderer, programs

- [ ] `invalidateState` and `resetGlState` overlap (docs point at both).
- [ ] `run_pass` is ~370 lines: the MSAA resolve and the invalidate list
  extract cleanly.
- [ ] Link polling written twice (`program_is_ready`, `program_link_pending`).
- [ ] Operation names in messages: snake_case vs camelCase; `poll_query`
  hardcodes "query_result".
- [ ] Extension names as strings beside the `Extension` enum; `Extension`
  not Copy.
- [ ] render_state.rs: `apply` repeats `apply_blend`; enable/disable ×7;
  two adjacent impl blocks; `0xFFFFFFFF` stencil mask ×4.
- [ ] stats.rs format sizes by raw hex (named in compressed_format.rs).
- [ ] param.rs `KIND_*` u32 with catch-all arms; `DrawCommand.kind` dead arm.
- [ ] Stale comments: trace tracks "≥ 2", fence comments, depth_range
  return, `describe_log` method nothing calls.
- [ ] profiler generation reset ×2; loss bookkeeping in context.rs;
  `epoch` vs `generation()`.
- [ ] JS predicate naming (`programIsReady` / `isFramebufferComplete` /
  `queryPending`; `ubo*` vs `UniformBuffer`) — breaking, to decide.
- [ ] program.rs six impl blocks; parked-sampler literal ×2; material
  slot `groups[1]` bare.

## Core: resources

- [ ] Doc comments on the wrong items (texture.rs ×2, framebuffer.rs
  `setResolutionScale` undocumented, texture_compressor.rs).
- [ ] Docs contradicting code (texture_compressor formats, ASTC/BPTC;
  copy.rs `dst_buffers`).
- [ ] Format tables ×3 (readback, framebuffer, float_targets).
- [ ] GL constants redefined (anisotropy ×3, RGBA8 …, bare hex) → `Gl::`.
- [ ] Anisotropy: sampler warns, texture sets blindly.
- [ ] copy.rs re-implements `context_attribute`; blit scissor block ×2.
- [ ] Attachment format looked up inline (use `source_format`); buffer
  release written out (use `vao::release_buffer`).
- [ ] vao_builder: dead `gl_type` field, `to_vertex_attr` re-match, eight
  `add_*` alike.
- [ ] texture_compressor: `preference` ignores its inputs; FORMATS lookup
  ×2, key ×4.
- [ ] `create_framebuffer` doesn't validate its size; `texture_storage_*`
  can't report failure; message prefixes vary.
- [ ] texture.rs upload duplication (sub-image ×5, image/bitmap load).
- [ ] `ClearMask` / `InvalidateMask` twins.
- [ ] DefaultFramebuffer getters (`clear_color_rgba` …) not mirrored or
  exported; framebuffers/renderbuffers lack `xIsValid`.
- [ ] `TextureTarget`, `BufferTarget`, `AttrType` not Copy; `BufferTarget`
  public but unused.
- [ ] "contains then destroy" teardown (compressor, packer).
- [ ] Encoder shaders share srgb/uniforms/load_block/main → common.glsl;
  fullscreen VERT ×3.
- [ ] Minor: `PixelUnpack::reset`, stale clippy allow, `format as u32`.

## Scene: rendering

- [ ] Fullscreen pass boilerplate ×9, program triple by hand, fullscreen
  vertex source ×6, `FULLSCREEN_VERT` in environment.rs → fullscreen.rs.
- [ ] `any_hidden` scan ×10 → one helper.
- [ ] output.rs (1636 lines): AutoExposure, Taa, Fxaa, PeelTargets split out.
- [ ] destroy conventions: `lost` / `live` / self-checking / pair.
- [ ] Depth format choice ×4 (one re-implements `depth_format()`);
  texture + framebuffer setup ×5; `mip_levels` ×3; direct.rs duplicates
  the MSAA-or-texture target.
- [ ] Stale block docs (`u_shading` z, w; `u_light_count` w), drifted
  between Rust and GLSL.
- [ ] Misplaced comments (render.rs ×2, output.rs FXAA, pbr.rs).
- [ ] render.rs: own-instance push ×2 and 4096 ×4; Dest match ×2;
  back-to-front key ×2; jitter matrix ×2; camera lookup ×2; two program
  caches.
- [ ] Lighting cache `Vec<Vec4>` by position → named.
- [ ] Shaders: skinning/morph between pbr.vert and motion.vert; surface.glsl
  re-implements `scene_reflection`; luma weights ×2.
- [ ] Visibility: `*_UNIFORM` pub but not exported; bare uniform strings;
  brdf_lut pub; `caster_bounds` / `scene_bounds` two names.
- [ ] `key_light` belongs with lights, not shadow views.
- [ ] Positional arrays to keep in sync (`FRAME_SAMPLERS`, `MAP_LAYERS`,
  `DROP_ORDER`).

## Scene: JS API

- [ ] Option helper errors lack the method name (every options setter).
  Non-finite numbers: console error + ignore vs throw — to decide.
- [ ] Getters: `quality()`, camera `aspect`, `mesh()`, environment size;
  `setMaterial` ↔ `meshMaterial`; background fill getter name;
  `setAutoExposureOptions` positional; getters.rs vs next to the setter.
- [ ] Options by hand (`zfar`; objects built without `object()`; PickHit ×2).
- [ ] Dead fallbacks after `finite()`; stale docs (speed, drawCalls,
  pending_variants ×2, `isTaa`/`isFxaa`, give_up_* docs, model_root_of,
  LoadedTexture, LossEvents).
- [ ] `ModelAnimations` exported, its field types not.
- [ ] Duplication: camera aspect sync ×2, variant error closure ×3,
  `instantiate` vs `own_entities`, Reloading loop ×2, map remap ×2,
  hidden-mesh filter ×2, parent walks ×3, `by_parent` ×2, look-at ×2,
  openpbr choice ×3.
- [ ] mod.rs (1500 lines): frame pipeline, models, picking → own files.
- [ ] `taa && hdr_wanted` ×4; fxaa / auto exposure pushed twice; display
  settings passed as 5 arguments ×2.
- [ ] Magic numbers: TAA settle 30, fill kinds 1/2/3, exposure defaults,
  "1 to 4" cascades, aspect fallback.
- [ ] `set_morph_weights`, `clear_parent` ignore a dead entity silently.
