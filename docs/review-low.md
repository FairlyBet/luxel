# Ревью low: результаты

263 low-находки ночного ревью (с добором), проверены агентами по группам и разобраны 20.11. Статусы: «исправлена» (в колонке — область, коммиты ниже), «исправлена раньше» — закрыта фиксами high/medium, «опровергнута», «пропущена» / «частично» — с причиной.

Итого: исправлена — 181, пропущена — 50, исправлена раньше — 25, опровергнута — 5, частично — 2.

Коммиты low-фиксов:
- примеры: ac901fe
- gltf: ffa3d72, db2b4a7
- gl: bd27c62, b4cb9c4 (там же clear_resolve сцены, 246/248)
- сцена (рендерер): f325d36
- фасад JS: 2cc3c78
- шейдеры, core, viewer, orbit, скрипты: d957cd1
- чтение канваса по drawing buffer (64), motion-вариант по атрибутам скина (241): последний коммит с этим файлом

Пропущенное «только производительность» — оптимизации без ошибки в поведении (лишние проходы, кэши, аллокации на кадр); их лучше делать с замерами.

| # | Файл | Находка | Статус | Заметка |
|---|---|---|---|---|
| 0 | `examples/cube/src/main.rs:106` | Each context restore leaks a param layout and group (also examples/scene) | исправлена | примеры |
| 1 | `examples/js-cube/serve.mjs:18` | Dev servers (js-cube, js-scene, npm-consumer serve.mjs) exit on a malformed request URL | исправлена раньше | вместе с high/medium |
| 2 | `examples/texture/src/main.rs:101` | Commented texture_load_image snippet doesn't compile | исправлена раньше | вместе с high/medium |
| 3 | `examples/viewer/main.js:273` | Two-finger gestures: a still two-finger tap picks, and coincident pointers jump the zoom to its limit | исправлена | viewer/orbit |
| 4 | `examples/viewer/main.js:457` | 'Save image' changes the framing when twice the canvas exceeds the capture limit | исправлена | viewer/orbit |
| 5 | `examples/viewer/main.js:692` | pickAt applies a pick result that can outlive the model, and has no rejection handling | исправлена | viewer/orbit |
| 6 | `examples/viewer/main.js:716` | Materials made by 'Give this part its own copy' are never destroyed | исправлена | viewer/orbit |
| 7 | `examples/viewer/profiling.js:47` | The long-task fallback never runs: observe() with an unsupported type does not throw | исправлена | viewer/orbit |
| 8 | `examples/viewer/profiling.js:124` | measureView restores settings blindly across awaits and not at all on failure | исправлена | viewer/orbit |
| 9 | `npm/README.md:265` | README lists isLost() among the scene's state queries; Scene has no such method | исправлена раньше | вместе с high/medium |
| 10 | `npm/orbit.js:113` | dispose() leaves `touch-action: none` on the element | исправлена | viewer/orbit |
| 11 | `npm/orbit.js:158` | onTap fires for right-click and shift-click, which are the pan gestures | исправлена | viewer/orbit |
| 12 | `luxel-core/src/column.rs:112` | Column sizes are multiplied in u32 without a check; an oversized resize wraps silently instead of failing | исправлена | core |
| 13 | `luxel-core/src/layout.rs:205` | Duplicate or colliding field paths are accepted silently | исправлена | core |
| 14 | `luxel-core/src/layout.rs:371` | measure() fully emits (and throws away) every nested leaf, repeated per struct-array element and per nesting l | пропущена | только производительность |
| 15 | `luxel-core/src/lib.rs:6` | Stale 'ECS components' claim for packed layouts; the Packed rule and the storage 'tracker clearing' comment de | исправлена | core |
| 16 | `luxel-ecs/src/storage.rs:72` | Storage's public write paths take any tick: no clamp to the world tick, and a stamp can lower a row's tick bel | опровергнута | The tick rule (docs/design/ecs.md, phase 2 item 5.1) has systems pass the `now` from advance_tick to Storage::get_mut / iter_mut / mark_changed, so passing worl |
| 17 | `luxel-ecs/src/storage.rs:244` | Mut::set_if_neq marks a component changed on every call once it holds a NaN | исправлена | core |
| 18 | `luxel-gl/src/buffer.rs:48` | Comment claims index buffers can bind to no other target; WebGL2 allows COPY_READ/COPY_WRITE, so the VAO unbin | пропущена | комментарий + VAO-unbind: менять поведение биндинга ради мелочи не стал |
| 19 | `luxel-gl/src/buffer.rs:119` | create_empty_buffer records a size the GL buffer does not have for sizes above i32::MAX (and when bufferData f | исправлена | gl |
| 20 | `luxel-gl/src/buffer.rs:186` | buffer_write does not check offset/length against the buffer size | исправлена | gl |
| 21 | `luxel-gl/src/compress/bc6h.frag:213` | BC6H: zero-range blocks and regions divide by zero; NaN disables the pattern search and feeds undefined index  | исправлена | gl |
| 22 | `luxel-gl/src/console.rs:47` | `*_once` dedup set grows without bound, and messages are formatted before the dedup check | исправлена | gl |
| 23 | `luxel-gl/src/context.rs:364` | `fromCanvas` reports every creation failure as "WebGL2 not supported" / "get_context failed" | исправлена | gl |
| 24 | `luxel-gl/src/context.rs:395` | ctx.setOnLost / setOnRestored cannot be cleared and swallow exceptions, unlike the scene's | исправлена | gl |
| 25 | `luxel-gl/src/context.rs:400` | Exceptions thrown by JS `setOnLost` / `setOnRestored` callbacks are silently discarded | исправлена | gl |
| 26 | `luxel-gl/src/context.rs:440` | `simulateContextRestore` returns true and discards the extension even when the restore cannot happen yet | исправлена | gl |
| 27 | `luxel-gl/src/context.rs:440` | simulateContextRestore returns true and throws the extension away even when the restore cannot happen yet | исправлена | gl |
| 28 | `luxel-gl/src/context.rs:532` | `Context::new` assumes a pristine, exclusively owned GL context | пропущена | Context::new на чужом контексте — архитектурное допущение, нужен отдельный режим |
| 29 | `luxel-gl/src/context_loss.rs:108` | Reverse-Z stays flagged on after a restore that no longer offers EXT_clip_control | исправлена | gl |
| 30 | `luxel-gl/src/context_loss.rs:108` | Reverse-Z stays flagged on after a restore that no longer offers EXT_clip_control | исправлена | gl |
| 31 | `luxel-gl/src/context_loss.rs:125` | Loss/restore listeners are never removed; every created context leaks two closures and its Core allocation | пропущена | слушатели потери не снимаются: нужен явный dispose у Context — не локально |
| 32 | `luxel-gl/src/context_loss.rs:125` | Loss / restore listeners are never removed and their closures are forgotten | исправлена | gl |
| 33 | `luxel-gl/src/context_loss.rs:125` | Loss / restore listeners are forgotten, never removed: each Context leaks two closures and its Core allocation | пропущена | дубль 31 |
| 34 | `luxel-gl/src/copy.rs:50` | Every blit issues synchronous GL getters (isEnabled, getContextAttributes), including the per-frame pass resol | опровергнута | isEnabled(SCISSOR_TEST) and getContextAttributes are answered from browser-side caches with no GPU round trip, and they run one to three times per frame on the  |
| 35 | `luxel-gl/src/copy.rs:50` | Every blit queries GL (isEnabled, getContextAttributes) on the per-frame path | пропущена | только производительность |
| 36 | `luxel-gl/src/copy.rs:396` | copy_texture_sub_image_2d forwards level, offsets and format to GL unvalidated | исправлена | gl |
| 37 | `luxel-gl/src/draw.rs:176` | instanceCount is silently ignored on rangeElements commands and the check for it is Rust-only | исправлена | gl |
| 38 | `luxel-gl/src/feedback.rs:170` | Recording into a buffer the same draw reads from is not checked, contrary to the "each rule is checked per dra | исправлена | gl |
| 39 | `luxel-gl/src/feedback.rs:231` | The transform feedback primitives query is not run when no draw is recorded, so it keeps the previous frame's  | исправлена | gl |
| 40 | `luxel-gl/src/float_targets.rs:108` | canRenderTo answers true for formats that are never color-renderable | исправлена | gl |
| 41 | `luxel-gl/src/framebuffer.rs:212` | Auto-resize observer outlives the Context while any DefaultFramebuffer handle is alive | исправлена | gl |
| 42 | `luxel-gl/src/framebuffer.rs:245` | Fallback size uses clientWidth/clientHeight (padding included, integer CSS px), not the content box | пропущена | fallback-размер по clientWidth: нужен getComputedStyle, затрагивает auto-resize |
| 43 | `luxel-gl/src/framebuffer.rs:409` | DefaultFramebuffer width()/height() are documented as the drawing-buffer size but return the viewport | опровергнута | By design: the canvas pass's viewport is its size, which auto-resize keeps current (otherwise the app does, via setViewport, as the fromGl doc says since the me |
| 44 | `luxel-gl/src/framebuffer.rs:419` | DefaultFramebuffer.setViewport takes the Viewport by value and consumes the JS object | исправлена раньше | вместе с high/medium |
| 45 | `luxel-gl/src/framebuffer.rs:1260` | Framebuffer size and default viewport are creation-time metadata, never reconciled with what is attached | пропущена | размер FBO как метаданные: сверка с вложениями — переделка модели FBO |
| 46 | `luxel-gl/src/gpu_info.rs:53` | ctx.limits() reports the first context's limits, not this context's | исправлена раньше | вместе с high/medium |
| 47 | `luxel-gl/src/interop.rs:26` | resetGlState does not restore the unpack parameters, and the cube-face upload relies on them | исправлена | gl |
| 48 | `luxel-gl/src/interop.rs:43` | `resetGlState` does an extension lookup and a reflective call every time, even when reverse-Z was never used | исправлена | gl |
| 49 | `luxel-gl/src/param.rs:83` | ParamLayoutBuilder and UboLayout consume themselves on each call; the other builders mutate in place | исправлена | gl |
| 50 | `luxel-gl/src/param.rs:141` | uniformArray count is unbounded: `components * count` overflows u32 or allocates gigabytes per group | исправлена | gl |
| 51 | `luxel-gl/src/pass.rs:64` | defaultKey returns a BigInt while pushWithKey takes two 32-bit numbers, and KEY_LAST is not exported | исправлена | gl |
| 52 | `luxel-gl/src/pass.rs:65` | defaultKey returns a bigint while pushWithKey takes the key as two u32 numbers | исправлена | gl |
| 53 | `luxel-gl/src/pass.rs:283` | A reused Pass cannot drop its viewport override or clear rect | исправлена | gl |
| 54 | `luxel-gl/src/pass.rs:317` | Pass::set_resolve keeps an earlier attachment list; the scene's reused frame pass resolves attachment 1 that n | исправлена | gl |
| 55 | `luxel-gl/src/poller.rs:70` | The no-`setTimeout` fallback fires rounds on the microtask queue, which can never let GPU results arrive | пропущена | только производительность |
| 56 | `luxel-gl/src/profiler.rs:79` | time() leaves the pass's previous timer query attached when no query is free | исправлена | gl |
| 57 | `luxel-gl/src/profiler.rs:139` | Timings of labels that are no longer timed are reported for ever | пропущена | профайлер хранит метки навсегда: нужна политика устаревания |
| 58 | `luxel-gl/src/profiler.rs:156` | destroy() after a context restore reports every old query as a stale handle | исправлена | gl |
| 59 | `luxel-gl/src/program.rs:444` | Shaders are deleted but never detached, so they live as long as the program | исправлена | gl |
| 60 | `luxel-gl/src/program.rs:488` | Bool compatibility rule ignores component count: an int vector field is silently accepted for a scalar bool un | исправлена | gl |
| 61 | `luxel-gl/src/program.rs:612` | Uniform-coverage check warns about parked samplers with an empty name, one warning per array element | исправлена | gl |
| 62 | `luxel-gl/src/program.rs:866` | programsReady resolves successfully when the context was lost and restored while it waited | исправлена | gl |
| 63 | `luxel-gl/src/program.rs:872` | programsReady resolves successfully when the context was lost and restored while it waited | исправлена | gl |
| 64 | `luxel-gl/src/readback.rs:192` | Canvas read region is validated against the default framebuffer's viewport, not the drawing buffer | исправлена | gl |
| 65 | `luxel-gl/src/readback.rs:276` | Async reads allocate a new GPU buffer per call and copy the bytes twice for JS | пропущена | только производительность |
| 66 | `luxel-gl/src/render_state.rs:61` | Constant blend factors are exposed but the blend colour can never be set | пропущена | blend color нельзя задать — это новая функция, не фикс |
| 67 | `luxel-gl/src/render_state.rs:446` | Polygon offset is not flipped under reverse-Z although the renderer claims to be transparent to render states | пропущена | polygon offset под reverse-Z: смена поведения, решать отдельно |
| 68 | `luxel-gl/src/render_state.rs:476` | Per-target blend is applied through getExtension and Reflect on every state change | пропущена | только производительность |
| 69 | `luxel-gl/src/renderbuffer.rs:36` | RenderbufferFormat::Stencil8 is exposed but there is no way to attach it | пропущена | Stencil8 нельзя прикрепить — нужен новый API |
| 70 | `luxel-gl/src/renderbuffer.rs:189` | Multisample renderbuffers record the requested sample count; 0 is silently turned into 1 | исправлена | gl |
| 71 | `luxel-gl/src/renderer.rs:241` | Every pass with a clear rect forgets the render state, forcing a full state re-apply per shadow tile | исправлена раньше | вместе с high/medium |
| 72 | `luxel-gl/src/renderer.rs:241` | A rect clear always forgets the render state: every shadow tile pass re-applies the full state | исправлена раньше | вместе с high/medium |
| 73 | `luxel-gl/src/renderer.rs:286` | Float-blend guard skips valid draws when per-target overrides turn blending off on the float32 attachment | пропущена | float-blend guard с per-target overrides — редкий случай, не локально |
| 74 | `luxel-gl/src/renderer.rs:372` | FrameStats counters use plain += and overflow if resetFrameStats is never called | исправлена | gl |
| 75 | `luxel-gl/src/renderer.rs:395` | Resolve and store ops call GL getters and allocate JS objects every frame | пропущена | только производительность |
| 76 | `luxel-gl/src/renderer.rs:411` | Default resolve path (no setResolveAttachments) skips the checks the other blit paths make | исправлена | gl |
| 77 | `luxel-gl/src/renderer.rs:411` | The default resolve path blits unchecked, then invalidates the source and moves the cache as if it had worked | исправлена | gl |
| 78 | `luxel-gl/src/renderer.rs:786` | Every draw re-diffs every field of every group, including groups shared by consecutive draws | пропущена | только производительность |
| 79 | `luxel-gl/src/renderer.rs:837` | Once-only warnings on the draw path format their message for every draw of every frame | пропущена | только производительность |
| 80 | `luxel-gl/src/shader_source.rs:80` | A chunk whose name equals the main file's name is silently never included | исправлена | gl |
| 81 | `luxel-gl/src/texture.rs:397` | Storage is recorded as allocated when GL rejects texStorage (wrong target, non-square cube, compressed format  | исправлена | gl |
| 82 | `luxel-gl/src/texture.rs:514` | 32F linear-filter warning does two synchronous getTexParameter reads and only looks at the filters at allocati | пропущена | предупреждение 32F-фильтра — только диагностика |
| 83 | `luxel-gl/src/texture.rs:760` | Every upload issues 18 pixelStorei calls, also when the unpack settings are the defaults | исправлена | gl |
| 84 | `luxel-gl/src/texture.rs:760` | Default uploads issue 18 pixelStorei calls and leave the active unit Unknown; the scene pays this every frame | пропущена | только производительность |
| 85 | `luxel-gl/src/texture.rs:825` | textureSubImage3DWith accepts flipY / premultiplyAlpha, which WebGL2 rejects for 3D uploads from an ArrayBuffe | исправлена | gl |
| 86 | `luxel-gl/src/texture.rs:881` | Cube-face upload is the only raw upload with no PixelUnpack and it neither applies nor resets unpack state | исправлена | gl |
| 87 | `luxel-gl/src/texture.rs:1536` | createImageBitmap path has no fallback to the <img> path when fetch or decode is refused | исправлена | gl |
| 88 | `luxel-gl/src/texture.rs:1591` | Exceptions thrown by the on_load callback are silently discarded | исправлена | gl |
| 89 | `luxel-gl/src/texture.rs:1785` | Non-UNSIGNED_BYTE uploads copy the whole pixel buffer a second time | пропущена | только производительность |
| 90 | `luxel-gl/src/texture_compressor.rs:266` | The 2D BC6H encoder is never precompiled and whenReady() does not cover it, so the first 2D Hdr compress block | пропущена | прекомпиляция 2D BC6H — расширение whenReady |
| 91 | `luxel-gl/src/texture_compressor.rs:440` | The 'one compressor per context' cache is a single slot: two contexts loading models evict each other's encode | пропущена | один слот компрессора на поток — переделка кэша |
| 92 | `luxel-gl/src/texture_compressor.rs:447` | SHARED holds one compressor for the whole thread, so two contexts evict each other and recompile all encoders | пропущена | дубль 91 |
| 93 | `luxel-gl/src/texture_compressor.rs:457` | One shared compressor slot for the thread: two contexts (or two scenes) evict each other and recompile every e | пропущена | один слот компрессора на поток — переделка кэша |
| 94 | `luxel-gl/src/texture_compressor.rs:764` | compress() issues 6-7 synchronous getTexParameter calls after queuing the encode draws | пропущена | только производительность |
| 95 | `luxel-gl/src/texture_compressor.rs:766` | compress() drops TEXTURE_MIN_LOD / TEXTURE_MAX_LOD although it promises to keep the sampler settings | исправлена | gl |
| 96 | `luxel-gl/src/texture_packer.rs:118` | PackSpec::set_source: the "fifth source" error branch is unreachable and its documented behaviour is not what  | исправлена | gl |
| 97 | `luxel-gl/src/texture_packer.rs:436` | pack()/packInto() return Ok with a blank texture when the draw is skipped for a stale source | исправлена | gl |
| 98 | `luxel-gl/src/trace.rs:310` | A finished recording can never be released; the event buffer keeps its capacity | пропущена | буфер законченной записи трейса — нужен API release |
| 99 | `luxel-gl/src/trace.rs:353` | gpu_fence does JS calls every frame when nothing is recording and no fences exist | исправлена | gl |
| 100 | `luxel-gl/src/trace.rs:396` | Fences and their WebGL context stay in the thread-local for ever once a context stops rendering | исправлена | gl |
| 101 | `luxel-gl/src/trace.rs:396` | Trace fences keep a clone of the WebGL context and its sync objects after the Context is dropped | исправлена | gl |
| 102 | `luxel-gl/src/trace.rs:656` | JS docs name TRACK_MAIN / TRACK_GPU, which are not exported to JS | исправлена | gl |
| 103 | `luxel-gl/src/uniform_buffer.rs:114` | UboLayout builder methods consume the JS object; the exported docs show Rust syntax and names | исправлена | gl |
| 104 | `luxel-gl/src/uniform_buffer.rs:215` | Uniform-buffer fields can only be written by path string: a string build/transfer and binary search per setter | пропущена | только производительность |
| 105 | `luxel-gl/src/uniform_buffer.rs:232` | UBO setters mark the field dirty even when the bytes did not change, so unchanged blocks are re-uploaded every | пропущена | только производительность |
| 106 | `luxel-gl/src/uniform_buffer.rs:266` | Block size is validated only after the whole layout is expanded: a large array count allocates one Field (with | исправлена раньше | вместе с high/medium |
| 107 | `luxel-gl/src/vao.rs:223` | vertex_array_attr does not validate size/stride/offset and records the attribute even when GL rejected it | исправлена | gl |
| 108 | `luxel-gl/src/vao_builder.rs:233` | VaoBuilder holds three copies of the vertex data at build time, and always interleaves vertex by vertex | пропущена | только производительность |
| 109 | `luxel-gl/src/vao_builder.rs:252` | setDivisor doc contradicts itself and a divisor for an index that is never added (or a matrix's later slot) is | исправлена | gl |
| 110 | `luxel-gl/src/vao_builder.rs:458` | VaoBuilder never checks the WebGL stride limit of 255 bytes; build succeeds with every pointer call rejected | исправлена | gl |
| 111 | `luxel-gltf/src/accessor.rs:81` | read_floats allocates and zero-fills count x components before checking the data can back it | исправлена | gltf |
| 112 | `luxel-gltf/src/animation.rs:106` | Dropped animation channels are silent: nothing reaches Model::warnings(), and empty clips stay in the list | исправлена | gltf |
| 113 | `luxel-gltf/src/buffers.rs:36` | Buffer byteLength is trusted: unchecked allocation for meshopt fallback buffers, u64 truncation, and no length | исправлена | gltf |
| 114 | `luxel-gltf/src/decoders.js:176` | Decoder workers and their wasm heaps are never released | пропущена | воркеры декодеров не освобождаются — нужен счётчик владельцев |
| 115 | `luxel-gltf/src/decoders.js:259` | Compressed inputs and the decoder wasm are copied more often than needed | исправлена | gltf |
| 116 | `luxel-gltf/src/decoders.js:259` | Every KTX2 file and Draco bufferView is copied twice before it is transferred to the worker | исправлена | gltf |
| 117 | `luxel-gltf/src/fetch.rs:24` | makeFetcher can throw through a non-catch import: loadModel throws synchronously and leaks the borrowed Contex | исправлена | gltf |
| 118 | `luxel-gltf/src/geometry.rs:209` | Unreadable or empty POSITION leaves an inverted infinite AABB and a primitive with no positions, giving the mo | исправлена | gltf |
| 119 | `luxel-gltf/src/geometry.rs:514` | Component count and element count of POSITION/NORMAL/TANGENT/TEXCOORD are never checked against what the packe | исправлена | gltf |
| 120 | `luxel-gltf/src/image_formats.rs:50` | WebP/AVIF support probe uses an <img> with a data: URI, which CSP img-src can block | пропущена | проба WebP/AVIF через data: — нужен другой способ пробы |
| 121 | `luxel-gltf/src/ktx2.rs:80` | KTX2 transcodes are not skipped or cancelled for a destroyed model or an aborted load | исправлена | gltf |
| 122 | `luxel-gltf/src/lib.rs:446` | Destroying a model (or despawning it, or destroying the scene) does not stop its image downloads and decodes | пропущена | уничтожение модели не останавливает загрузку картинок — нужен AbortSignal сквозь загрузчик |
| 123 | `luxel-gltf/src/lib.rs:478` | Model.adopt (exported to JS) overwrites the model's GPU handles without checking that they are lost: on a live | исправлена | gltf |
| 124 | `luxel-gltf/src/lib.rs:716` | loadModel validates its options differently from every scene option object | исправлена | gltf |
| 125 | `luxel-gltf/src/lib.rs:1034` | A .gltf that starts with a UTF-8 BOM is rejected | исправлена | gltf |
| 126 | `luxel-gltf/src/lib.rs:1046` | document() deep-clones the whole JSON although the caller never uses it again | исправлена | gltf |
| 127 | `luxel-gltf/src/loader.js:36` | Without `AbortSignal.any`, `cancel()` stops nothing when the app gave a signal, and a second signal is dropped | исправлена | gltf |
| 128 | `luxel-gltf/src/loader.js:156` | `onProgress` reports `total == loaded` for responses without Content-Length, though the docs say `total` is 0  | исправлена | gltf |
| 129 | `luxel-gltf/src/loader.js:185` | A dropped file whose URI has a bare '%' is never found in `files` and is fetched from https://files.invalid/ i | исправлена | gltf |
| 130 | `luxel-gltf/src/material.rs:260` | JS setter `openPbrNative` on Material only changes a detached copy | исправлена | gltf |
| 131 | `luxel-gltf/src/material.rs:260` | Material.openPbrNative has a JS setter that only changes a throwaway copy | исправлена | gltf |
| 132 | `luxel-gltf/src/material.rs:350` | Property `iridescenceThickness` does not use the glTF name the table promises | исправлена | gltf |
| 133 | `luxel-gltf/src/material.rs:422` | Reading one property clones the whole Material (heap-allocating its name) because only values_mut exists | исправлена | gltf |
| 134 | `luxel-gltf/src/material.rs:669` | After assign_uv_slots, map UV values are shader slots, but set_map and the JS getters document them as TEXCOOR | исправлена | gltf |
| 135 | `luxel-gltf/src/meshopt.rs:702` | meshopt view parameters are cast and added unchecked, unlike json_view | исправлена | gltf |
| 136 | `luxel-gltf/src/pack.rs:8` | Module doc says a packed material samples one texture/unit; the shader still samples two | исправлена | gltf |
| 137 | `luxel-gltf/src/pack.rs:59` | A new TexturePacker (program, FBO, dummy texture, sampler, layout) is built for every model load with an ORM p | пропущена | TexturePacker на каждую модель — перф, кэш между моделями |
| 138 | `luxel-gltf/src/pack.rs:320` | Transient ORM source textures get a full mip chain that is thrown away | исправлена | gltf |
| 139 | `luxel-gltf/src/pack.rs:408` | When both ORM sources fail, the packed texture is left without storage and reads as fully occluded | исправлена | gltf |
| 140 | `luxel-gltf/src/pack.rs:436` | ORM packing turns GPU-compressed KTX2 sources into an RGBA8 texture | исправлена | gltf |
| 141 | `luxel-gltf/src/scene.rs:139` | Model bounding sphere includes meshes under KHR_node_visibility-hidden nodes | исправлена | gltf |
| 142 | `luxel-gltf/src/scene.rs:227` | EXT_mesh_gpu_instancing with normalized BYTE/SHORT rotations (allowed by the extension) collapses to a single  | исправлена | gltf |
| 143 | `luxel-gltf/src/scene.rs:246` | Dead length check and stale doc comment in gpu_instances | исправлена раньше | вместе с high/medium |
| 144 | `luxel-gltf/src/textures.rs:180` | KTX2 detection by URI extension is case-sensitive | исправлена | gltf |
| 145 | `luxel-gltf/src/vertex_format.rs:143` | Float normals/tangents are always stored as SNORM16, clamping non-unit vectors per component and changing thei | исправлена | gltf |
| 146 | `luxel-gltf/src/vertex_format.rs:190` | encode() clamps non-normalized signed integers to -127 / -32767, so the type's minimum value is altered | исправлена | gltf |
| 147 | `luxel/shaders/env_bake.frag:107` | Mipmapped equirect lookup over-blurs the poles of the baked cube | пропущена | полюса equirect-запекания: нужна LOD по телесному углу и визуальная проверка |
| 148 | `luxel/shaders/exposure.frag:66` | Exposure easing overshoots and oscillates when speed x dt exceeds 1 | исправлена раньше | вместе с high/medium |
| 149 | `luxel/shaders/gtao.frag:42` | Stale uniform documentation and dead code in the post shaders | исправлена | шейдеры |
| 150 | `luxel/shaders/gtao_denoise.frag:29` | Every depth read of the denoise (and the half-res GTAO centre) lands exactly on a texel corner of a NEAREST de | пропущена | только производительность |
| 151 | `luxel/shaders/lib/lights.glsl:49` | Lighting block comment says u_output.z is 1 with a scene copy; it is 1 + the least mip to read | исправлена | шейдеры |
| 152 | `luxel/shaders/lib/subsurface.glsl:27` | Subsurface profile integration repeats 32 light-independent profile evaluations per light | пропущена | только производительность |
| 153 | `luxel/shaders/lib/surface.glsl:69` | Includable chunk surface.glsl depends on a varying it neither declares nor lists in its "Needs" | исправлена | шейдеры |
| 154 | `luxel/shaders/lib/surface_openpbr.glsl:293` | Shadow maps are sampled for lights that contribute nothing to the pixel | исправлена | шейдеры |
| 155 | `luxel/shaders/pbr.frag:269` | Zero-length TANGENT or NORMAL produces NaN pixels, which nothing downstream contains | исправлена | шейдеры |
| 156 | `luxel/shaders/pbr.vert:119` | `u_normal_matrix` macro re-reads the instance and inverts a 3x3 per vertex, although the model matrix was just | исправлена | шейдеры |
| 157 | `luxel/shaders/skybox.frag:23` | Environment background under an orthographic camera uses 1/P00 as a field of view | исправлена | шейдеры |
| 158 | `luxel/src/animation.rs:130` | sample() underflows and panics when a channel's first keyframe time is NaN | исправлена раньше | вместе с high/medium |
| 159 | `luxel/src/brdf_lut.rs:18` | BRDF LUT is integrated on the main thread at every renderer creation and context restore | исправлена | сцена |
| 160 | `luxel/src/culling.rs:28` | Infinite, huge or inverted Bounds make a mesh always culled instead of never culled | исправлена | сцена |
| 161 | `luxel/src/environment.rs:103` | Environment::bake silently bakes R11F_G11F_B10F cubes; docs say RGBA16F and there is no way to choose | исправлена | сцена |
| 162 | `luxel/src/environment.rs:497` | A bake 'stage' is all six faces of a mip, so large sizes put hundreds of millions of cube samples in one step | пропущена | только производительность |
| 163 | `luxel/src/environment.rs:546` | PendingEnvironment dropped without cancel leaks the bake's result cubes (public Rust API) | исправлена | сцена |
| 164 | `luxel/src/environment.rs:578` | A bake overtaken by a loss and restore destroys handles of the dead generation and logs stale-handle errors | исправлена | сцена |
| 165 | `luxel/src/environment.rs:584` | Stage spacing uses the wall clock (Date.now), which is not monotonic | исправлена | сцена |
| 166 | `luxel/src/environment.rs:636` | cancel() / the task destroy handles from a lost generation after the context was restored, logging stale-handl | исправлена | сцена |
| 167 | `luxel/src/environment.rs:636` | Destroy paths that are not generation-aware log stale-handle errors after a restore | исправлена | сцена |
| 168 | `luxel/src/hdr.rs:68` | Header parser does not accept CRLF line endings | исправлена | сцена |
| 169 | `luxel/src/hdr.rs:97` | decode_hdr_rgbe allocates and zero-fills width*height*4 from the header before looking at how much pixel data  | исправлена | сцена |
| 170 | `luxel/src/hdr.rs:203` | encode_hdr mishandles infinite/huge values and does not check rgb.len() against width*height | исправлена | сцена |
| 171 | `luxel/src/instances.rs:105` | The batch index reallocates one Vec per batch key every frame | исправлена | сцена |
| 172 | `luxel/src/instances.rs:202` | Growing the instance texture past MAX_TEXTURE_SIZE destroys the working texture and never recovers | исправлена | сцена |
| 173 | `luxel/src/instances.rs:208` | Instance upload does three full passes over the data every frame anything moves | пропущена | только производительность |
| 174 | `luxel/src/js/animation.rs:196` | setAnimationSpeed is silently dropped unless a clip is playing, and playAnimation resets it to 1 | исправлена | фасад |
| 175 | `luxel/src/js/animation.rs:196` | setAnimationSpeed is silently dropped when nothing plays and reset by every play/crossfade | исправлена | фасад |
| 176 | `luxel/src/js/animation.rs:221` | advanceAnimations ignores a non-finite dt without the report every other setter gives | исправлена | фасад |
| 177 | `luxel/src/js/capture.rs:40` | capture is documented to reject without a canvas or with a lost context, but throws synchronously | исправлена | фасад |
| 178 | `luxel/src/js/capture.rs:105` | capture replaces a viewport the app set on the default framebuffer with the full drawing buffer | исправлена | фасад |
| 179 | `luxel/src/js/getters.rs:26` | exposure() is documented 'in stops' but is a linear multiplier | исправлена | задокументировано |
| 180 | `luxel/src/js/lights.rs:49` | setCamera validates znear/zfar but not yfov / ymag: degenerate values give an infinite or flipped projection | исправлена | фасад |
| 181 | `luxel/src/js/lights.rs:281` | Option getters read back unclamped values and some options are not range-checked | частично | проверки yfov/ymag/intensity/range/splitLambda и размер тени на земле сделаны; shadowMapSize света по-прежнему возвращается как задан |
| 182 | `luxel/src/js/lights.rs:394` | Restore work done twice or needlessly: old environment re-baked then cancelled, HDR bytes cloned, BRDF table r | частично | частично: таблица BRDF теперь кэшируется (159); повторное запекание/клон байтов при restore оставлено |
| 183 | `luxel/src/js/lights.rs:396` | setEnvironmentHdr (and loadTexture / createProgram) during a loss throw and are not recorded, so the restore d | исправлена | фасад |
| 184 | `luxel/src/js/lights.rs:476` | setBackground / setBackgroundColor / setTaa create GL programs without sync_context: one program leaks after a | исправлена | фасад |
| 185 | `luxel/src/js/materials.rs:223` | setMaterialProperty rejects Infinity/NaN with a misleading message, making glTF's attenuationDistance = Infini | исправлена | фасад |
| 186 | `luxel/src/js/materials.rs:223` | setMaterialProperty rejects Infinity for attenuationDistance, which glTF and the Rust setter treat as 'no abso | исправлена | фасад |
| 187 | `luxel/src/js/materials.rs:381` | A failed loadTexture stays in the scene's texture list and is fetched again after every context loss | исправлена раньше | вместе с high/medium |
| 188 | `luxel/src/js/materials.rs:381` | A loadTexture that failed stays in scene.textures and is fetched and re-created after every context loss | исправлена раньше | вместе с high/medium |
| 189 | `luxel/src/js/materials.rs:381` | A loadTexture that fails stays in the scene's texture list and is fetched again after every context loss | исправлена раньше | вместе с high/medium |
| 190 | `luxel/src/js/materials.rs:506` | Rust `set_material_values` ("setMaterialProperty from Rust") does not invalidate shadows like the JS path does | исправлена | фасад |
| 191 | `luxel/src/js/materials.rs:520` | Every material property write rescans all meshes and re-resolves variant, vertex array and shadow caster for e | пропущена | только производительность |
| 192 | `luxel/src/js/mod.rs:199` | Constructing a Scene changes how createProgram reports errors on the shared context | исправлена | задокументировано |
| 193 | `luxel/src/js/mod.rs:291` | despawn frees a model's GPU data while entities reparented out of its subtree still draw with it | пропущена | despawn и перенесённые setParent'ом узлы: нужна привязка примитивов к модели, не к иерархии |
| 194 | `luxel/src/js/mod.rs:328` | setRotation(0,0,0,0) normalizes a zero quaternion to NaN | исправлена | фасад |
| 195 | `luxel/src/js/mod.rs:385` | lookAt with eye == target (or non-finite input) writes a NaN transform | исправлена раньше | вместе с high/medium |
| 196 | `luxel/src/js/mod.rs:556` | removeMesh does not rebake frozen shadows (setMesh, despawn and setVisible do) | исправлена | фасад |
| 197 | `luxel/src/js/mod.rs:574` | setBounds accepts NaN / inverted boxes: the mesh is then silently culled forever | исправлена раньше | вместе с high/medium |
| 198 | `luxel/src/js/mod.rs:595` | createProgram records are never dropped: destroyed shaders are recompiled after every context loss and leak wi | исправлена раньше | вместе с high/medium |
| 199 | `luxel/src/js/mod.rs:713` | addModelWithProgram does not validate the program (setMesh does) | исправлена | фасад |
| 200 | `luxel/src/js/mod.rs:970` | scene.setReverseZ and ctx.setReverseZ return opposite values for the same call | опровергнута | Scene.setReverseZ's doc says it returns whether reverse-Z is on, like the scene's setHdr, setTaa and setFxaa; the context's convention is a separate, recorded d |
| 201 | `luxel/src/js/mod.rs:1162` | gpuTimings 'glass' is one layer's time, not the frame's, and eats the profiler ring; docs list only three labe | исправлена | задокументировано |
| 202 | `luxel/src/js/mod.rs:1236` | A Scene dropped without destroy() (free() or weak-ref GC) leaks everything and leaves background work 'alive' | исправлена раньше | вместе с high/medium |
| 203 | `luxel/src/js/mod.rs:1236` | A Scene dropped without destroy() (free() or the weak-ref finalizer) frees nothing and leaves its background w | исправлена раньше | вместе с high/medium |
| 204 | `luxel/src/js/on_demand.rs:91` | Scene.render() shares the on-demand state: with TAA and a second camera needsRender never becomes false | исправлена раньше | вместе с high/medium |
| 205 | `luxel/src/js/openpbr.rs:212` | geometry_opacity is silently a no-op on materials loaded with alphaMode OPAQUE | исправлена | фасад |
| 206 | `luxel/src/js/options.rs:154` | Handles<T> gives the brand only on index access; iteration yields plain numbers | исправлена | фасад |
| 207 | `luxel/src/js/options.rs:227` | u32_or's error calls every integer option "a handle" | исправлена | фасад |
| 208 | `luxel/src/js/options.rs:264` | Finite check is done on the f64, then cast to f32: large finite numbers become Infinity and pass | исправлена | фасад |
| 209 | `luxel/src/js/output.rs:75` | Turning ambient occlusion and contact shadows off never frees the occlusion passes and depth copy | исправлена | фасад |
| 210 | `luxel/src/js/output.rs:243` | Setters called between the restore and the scene's first sync create GL objects inside structures that are the | исправлена | фасад |
| 211 | `luxel/src/js/output.rs:421` | prepare_output scans every mesh against every PBR variant each frame, even with contact shadows off | исправлена | фасад |
| 212 | `luxel/src/js/output.rs:421` | prepare_output scans every mesh against every PBR variant each frame, even with contact shadows off | исправлена | фасад |
| 213 | `luxel/src/js/output.rs:596` | apply_occlusion walks all shadow casters once or twice per frame for values that rarely change | пропущена | только производительность |
| 214 | `luxel/src/js/picking.rs:25` | pick, worldToScreen, screenToRay and raycastBounds use a stale camera aspect | исправлена | фасад |
| 215 | `luxel/src/js/picking.rs:29` | pick / worldToScreen / screenToRay / raycastBounds use a stale camera aspect (1 for a camera not rendered yet) | исправлена | фасад |
| 216 | `luxel/src/js/restore.rs:98` | Cancelling a finished-but-unapplied bake at restore destroys stale handles and logs errors | исправлена | фасад |
| 217 | `luxel/src/js/restore.rs:141` | createProgram shaders the app destroyed are kept, recompiled after a loss and owned by nobody | исправлена раньше | вместе с high/medium |
| 218 | `luxel/src/js/restore.rs:199` | Textures whose loadTexture failed are re-created and re-fetched at every restore | исправлена раньше | вместе с high/medium |
| 219 | `luxel/src/js/restore.rs:238` | No currentTexture for JS although loadTexture handles change after a loss | исправлена | фасад |
| 220 | `luxel/src/js/restore.rs:293` | Warning blames setRestoreModels(false) when the model simply has no source | исправлена | фасад |
| 221 | `luxel/src/js/restore.rs:342` | A reload landing does not re-arm the TAA settle count: the model is drawn for one TAA frame only | исправлена раньше | вместе с high/medium |
| 222 | `luxel/src/js/restore.rs:342` | A reload (or environment bake) landing inside a frame does not re-arm the TAA settle count | исправлена раньше | вместе с high/medium |
| 223 | `luxel/src/js/restore.rs:349` | apply_reloads reports a reload aborted by a second loss as an error: its `!ctx.is_lost()` guard is always true | исправлена | фасад |
| 224 | `luxel/src/js/restore.rs:349` | A reload that failed because of a second loss is reported as a model error once the context is back | исправлена | фасад |
| 225 | `luxel/src/js/restore.rs:458` | setOnRestored doc says needsRender triggers the rebuild; it does not | исправлена | задокументировано |
| 226 | `luxel/src/lighting/cache.rs:68` | Signatures walk every joint of a skin once per primitive per view, every frame | пропущена | только производительность |
| 227 | `luxel/src/lighting/casters.rs:54` | Per-caster visibility, link and reload checks are repeated for every shadow view | исправлена | сцена |
| 228 | `luxel/src/lighting/casters.rs:63` | Waiting casters are counted once per shadow view, inflating stats.pending | исправлена | сцена |
| 229 | `luxel/src/lighting/ground.rs:148` | The ground shadow's darkness and blur programs are not in the readiness list: first use can stall on a synchro | исправлена | сцена |
| 230 | `luxel/src/lighting/ground.rs:258` | Ground shadow textures stay allocated after the ground shadow is turned off | исправлена | сцена |
| 231 | `luxel/src/lighting/ground.rs:261` | GroundShadow.size is silently clamped and rounded, and the getter reports the unclamped value | исправлена | сцена |
| 232 | `luxel/src/lighting/maps.rs:315` | A render state is interned per view per frame by linear search, and bias edits leave states behind | пропущена | только производительность |
| 233 | `luxel/src/lighting/mod.rs:301` | prepare() doc says only directional / spot lights cast shadows | исправлена | сцена |
| 234 | `luxel/src/lighting/mod.rs:358` | Every distinct bias pair interns a render state that is never released | опровергнута | Interned render states that are never freed are the core's accepted design (medium #16 kept interning and fixed only the scissor and NaN cases); new states come |
| 235 | `luxel/src/lighting/mod.rs:502` | Lights past MAX_LIGHTS still get shadow maps planned, drawn and stored | исправлена раньше | вместе с high/medium |
| 236 | `luxel/src/lighting/pack.rs:84` | LightsBlock::write rescans every caster each frame via key_light, also on frozen frames | пропущена | только производительность |
| 237 | `luxel/src/lighting/settings.rs:6` | LightShadow docs still say only directional and spot lights cast shadows | исправлена | сцена |
| 238 | `luxel/src/lighting/views.rs:201` | Spot (and point) light with a range shorter than the nearest caster builds an inverted/empty shadow frustum in | исправлена | сцена |
| 239 | `luxel/src/lighting/views.rs:521` | caster_bounds collapses to a point for casters without Bounds and for single-joint skins, so the directional f | пропущена | caster_bounds для мешей без Bounds/одного сустава — нужна оценка границ скина |
| 240 | `luxel/src/motion.rs:167` | track() rebuilds every mesh's state each frame, including a second full joint-matrix pass per skinned primitiv | исправлена | сцена |
| 241 | `luxel/src/motion.rs:219` | Motion variant is chosen from the Skin component, not the primitive's skin attributes | исправлена | сцена |
| 242 | `luxel/src/occlusion.rs:312` | Occlusion targets and programs stay allocated after AO and contact shadows are turned off | исправлена | сцена |
| 243 | `luxel/src/output.rs:510` | set_fxaa / set_auto_exposure on an HdrOutput of a lost generation leak the new GL objects (or destroy stale ha | исправлена | сцена |
| 244 | `luxel/src/output.rs:510` | setFxaa / setAutoExposure between a restore and the next frame build GL objects into an HdrOutput of the lost  | исправлена | сцена |
| 245 | `luxel/src/output.rs:605` | Per-frame uniform/texture sets by name in the hot path | пропущена | только производительность |
| 246 | `luxel/src/output.rs:716` | The reused frame pass keeps resolve attachments [0, 1] after contact shadows' key output goes away (MSAA) | исправлена | сцена |
| 247 | `luxel/src/output.rs:717` | Stale resolve attachment list [0, 1] left on the opaque pass after the key-light output goes away (MSAA) | исправлена | сцена |
| 248 | `luxel/src/output.rs:717` | Reused frame pass keeps resolve attachments [0, 1] after the key-light output goes away | исправлена | сцена |
| 249 | `luxel/src/pbr.rs:197` | fit_texture_units doc says the frame uses four samplers; the code reserves six | исправлена | сцена |
| 250 | `luxel/src/pbr.rs:247` | shadow_key keeps normal and UV attribute bits for opaque materials, splitting shadow programs | исправлена | сцена |
| 251 | `luxel/src/pbr.rs:333` | PBR variants (and their depth-only twins) are never released while the scene lives, whatever is despawned | пропущена | PBR-варианты не освобождаются — нужен подсчёт ссылок на варианты |
| 252 | `luxel/src/pbr.rs:402` | is_pbr is a linear scan and runs for every mesh on every HDR frame | исправлена | сцена |
| 253 | `luxel/src/render.rs:98` | The public LIGHTING_BLOCK doc for u_output.z disagrees with what is written | исправлена | сцена |
| 254 | `luxel/src/render.rs:777` | Early return on a bad camera leaves last frame's shadow passes to be rendered again, possibly with freed objec | исправлена | сцена |
| 255 | `luxel/src/render.rs:1165` | Shadow casters with instance_count above MAX_COPIES draw unclamped and read other objects' matrices | исправлена | сцена |
| 256 | `luxel/src/render.rs:1225` | Joint matrices are recomputed and written once per primitive, not once per skin | пропущена | только производительность |
| 257 | `luxel/src/render.rs:1519` | pick() draws meshes that are still Reloading after a context restore, using stale vertex arrays | исправлена | сцена |
| 258 | `luxel/src/render.rs:1519` | pick() draws meshes marked Reloading, whose vertex arrays are from the lost context | исправлена | сцена |
| 259 | `luxel/src/render.rs:1519` | pick() draws meshes that are still Reloading, with vertex arrays of the lost context | исправлена | сцена |
| 260 | `luxel/src/render.rs:1519` | pick() draws meshes that are still Reloading, with vertex arrays of the lost context | исправлена | сцена |
| 261 | `scripts/brand-handles.mjs:413` | Brand declarations are spliced between the first export and its JSDoc | исправлена | скрипты |
| 262 | `scripts/typings-check.ts:41` | Material map, property and semantics names and gpuTimings are untyped; the typings check itself passes a name  | исправлена | скрипты |
