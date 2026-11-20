# Ревью medium: результаты проверки

Ночные medium-находки (97) после дедупликации проверены семью агентами по коду (20.11). Статусы: «исправлена <коммит>», «открыта» — подтверждена, ждёт фикса, «решить» — нужен твой выбор, «не чинить».

Плюс три medium из добора по примерам: js-cube — цикл после потери (исправлена 523908d), tsc в README js-cube/js-scene (исправлена fc794a6).

| # | Файл | Находка | Проверка | Статус | Заметка |
|---|---|---|---|---|---|
| 0 | `npm/orbit.js:70` | OrbitControls never places the camera until the first input; writes to target/distance/yaw/pitch are ignored | опровергнута | — | update(dt, true) ставит камеру; так задумано |
| 1 | `luxel-core/src/layout.rs:414` | Array / struct-array counts are unvalidated: a bad count from JS hangs or OOMs the tab before any size check | подтверждена | исправлена 25a49e2 | счётчик массива из JS не проверяется до build(): зависание/OOM |
| 2 | `luxel-gl/src/context.rs:352` | Context.fromGl leaves the canvas framebuffer at a 0×0 viewport | подтверждена | исправлена 5b82c2f |  |
| 3 | `luxel-gl/src/context.rs:353` | `fromGl` leaves the default framebuffer with a 0x0 viewport and no canvas | дубль 2 | исправлена 5b82c2f |  |
| 4 | `luxel-gl/src/copy.rs:206` | blit_framebuffer skips the format and same-image checks, so rejected blits end as silent GL errors | подтверждена | исправлена 00de309 | blit без проверки форматов/same-fb: GL-ошибка без объяснения |
| 5 | `luxel-gl/src/copy.rs:374` | copy_texture_sub_image_2d refuses an antialiased canvas source, which WebGL allows | подтверждена | исправлена 5b82c2f |  |
| 6 | `luxel-gl/src/feedback.rs:352` | Transform feedback binding accepts sizes that are not a multiple of 4; GL rejects them and luxel still records | подтверждена | исправлена 5b82c2f |  |
| 7 | `luxel-gl/src/framebuffer.rs:290` | Auto-resize sets canvas.width/height inside the ResizeObserver callback, so resized frames are presented blank | подтверждена (не проверено в браузере) | исправлена 760fe8a | ресайз применяется перед отрисовкой; мерцание подтверждено в Chrome и ушло |
| 8 | `luxel-gl/src/framebuffer.rs:419` | DefaultFramebuffer.setViewport takes its Viewport by value and consumes the JS object | подтверждена | исправлена 5b82c2f |  |
| 9 | `luxel-gl/src/framebuffer.rs:447` | Auto-resize on a canvas without a CSS size grows the canvas without bound on HiDPI screens | подтверждена | исправлена 11a8b3c | канвас без CSS-размера растёт на HiDPI; доки + предупреждение |
| 10 | `luxel-gl/src/framebuffer.rs:1065` | Invalidating only depth (or only stencil) on a depth-stencil attachment discards both | подтверждена | исправлена 5b82c2f |  |
| 11 | `luxel-gl/src/framebuffer.rs:1065` | Invalidating depth alone on a depth-stencil attachment discards the stencil too (and the reverse) | дубль 10 | исправлена 5b82c2f |  |
| 12 | `luxel-gl/src/framebuffer.rs:1128` | Attachments are recorded and the previous one released before knowing whether GL accepted the attach | подтверждена | исправлена 82e22f1 | attach текстуры не того типа: пул расходится с GL |
| 13 | `luxel-gl/src/limits.rs:49` | Limits are frozen from the first context ever created and never re-queried | подтверждена | исправлена adc861a | без теста: лимиты глобальны на страницу |
| 14 | `luxel-gl/src/param.rs:552` | paramSetBlockRange stores any offset/size unvalidated; a bad range fails at draw and is cached as bound | подтверждена | исправлена 82e22f1 | paramSetBlockRange без проверки диапазона |
| 15 | `luxel-gl/src/program.rs:181` | All parked samplers share one texture unit, so two unsupported sampler arrays of different types fail every dr | подтверждена | исправлена 25a49e2 | запаркованные sampler-массивы разных типов на одном юните |
| 16 | `luxel-gl/src/render_state.rs:687` | Interned render states are never freed and are found by a linear scan, so per-frame-varying state grows withou | подтверждена | исправлена 3f4dff0 | scissor перенесён в Pass; NaN-offset → 0 |
| 17 | `luxel-gl/src/shader_source.rs:80` | Include-once ignores preprocessor conditionals: a chunk first included inside a false #if is dropped everywher | подтверждена | задокументирована 4d8ad29 | include-once не видит #if |
| 18 | `luxel-gl/src/texture.rs:1679` | Image load with generate_mipmaps allocates a full mip chain before checking the format can have mipmaps genera | подтверждена | исправлена 5b82c2f |  |
| 19 | `luxel-gl/src/texture_compressor.rs:440` | The shared texture compressor pins its Context for the life of the page; only Scene::destroy lets go, and a la | подтверждена частично | частично 82e22f1 | компрессор создаётся только когда есть что сжимать; один контекст всё ещё держится до Scene.destroy |
| 20 | `luxel-gl/src/trace.rs:298` | start() keeps fences from the previous recording, giving a GPU span with a stale timestamp and negative durati | подтверждена | исправлена 5b82c2f |  |
| 21 | `luxel-gl/src/trace.rs:376` | GPU fence spans partially overlap on one track exactly when the GPU falls behind | подтверждена | исправлена 5b82c2f |  |
| 22 | `luxel-gl/src/trace.rs:666` | Trace.complete with a non-finite time makes the whole report invalid JSON | подтверждена | исправлена 5b82c2f |  |
| 23 | `luxel-gl/src/vao_builder.rs:378` | buildInto on an already configured vertex array keeps the old buffers and stale attributes until the VAO is de | подтверждена | исправлена 82e22f1 | buildInto повторно: старые буферы и атрибуты остаются |
| 24 | `luxel-gltf/src/animation.rs:66` | Animation and instancing accessors are read with the gltf crate's unchecked iterators: byteStride smaller than | подтверждена | исправлена 5f093f1 | анимации/инстансинг читаются небезопасным итератором gltf: паника |
| 25 | `luxel-gltf/src/animation.rs:81` | Animation output accessor with a spec-forbidden component type traps the loader (gltf crate unreachable!) | подтверждена | исправлена 5f093f1 | запрещённый тип выхода анимации → unreachable! в gltf |
| 26 | `luxel-gltf/src/animation.rs:102` | Keyframe times are not validated (NaN / non-increasing) although the API promises "increasing"; a NaN first ke | подтверждена | исправлена 5f093f1 | NaN/неупорядоченные времена ключей → паника |
| 27 | `luxel-gltf/src/decoders.js:68` | A decoder whose wasm fails to instantiate leaves every job pending forever | подтверждена (по документации Emscripten) | исправлена 5f093f1 | сбой wasm декодера → вечное ожидание |
| 28 | `luxel-gltf/src/decoders.js:68` | A decoder wasm that fails to instantiate leaves every job, whenLoaded, whenReady and loadModel pending forever | дубль 27 | исправлена 5f093f1 |  |
| 29 | `luxel-gltf/src/decoders.js:233` | setDecoderPath always closes the live pool, failing jobs in flight even when the path is unchanged | подтверждена | исправлена 5f093f1 | setDecoderPath с тем же путём рвёт текущие задачи |
| 30 | `luxel-gltf/src/decoders.js:233` | setDecoderPath closes the live pool: transcodes and Draco decodes in flight fail, even when the path is unchan | дубль 29 | исправлена 5f093f1 |  |
| 31 | `luxel-gltf/src/fetch.rs:52` | Every fetched file is copied twice on its way into wasm (extra full-size JS copy) | подтверждена | исправлена 5f093f1 | лишняя копия каждого файла при загрузке |
| 32 | `luxel-gltf/src/ktx2.rs:146` | A KTX2 texture larger than MAX_TEXTURE_SIZE is reported as loaded but has no storage | подтверждена | исправлена 5f093f1 | KTX2 больше MAX_TEXTURE_SIZE «загружен», но пуст |
| 33 | `luxel-gltf/src/material.rs:582` | Alpha-only maps are flagged linear, so a texture shared with its colour map is decoded and uploaded twice | подтверждена | исправлена e4c5bf4 | alpha-only карты дублируют текстуру |
| 34 | `luxel-gltf/src/pack.rs:80` | Textures no material slot reads are still fetched, decoded and kept as uncompressed RGBA8 with mips | подтверждена | исправлена 5f093f1 | неиспользуемые текстуры грузятся |
| 35 | `luxel-gltf/src/pack.rs:445` | empty_callback() uses `new Function("")`, which a CSP without 'unsafe-eval' blocks | подтверждена | исправлена aa793b0 |  |
| 36 | `luxel-gltf/src/scene.rs:173` | Node walk has no cycle or depth guard: a cyclic node graph recurses until the wasm stack overflows | подтверждена | исправлена 5f093f1 | цикл в графе узлов → переполнение стека |
| 37 | `luxel-gltf/src/scene.rs:239` | gpu_instances silently substitutes defaults when an instancing attribute can't be read; the "data out of range | подтверждена | исправлена 5f093f1 | инстансинг: обрезанные данные без предупреждения |
| 38 | `luxel/shaders/exposure.frag:46` | One non-finite pixel drives auto exposure to its limit, or to NaN for good | подтверждена | исправлена 2b9fca4 | Inf в кадре ломает автоэкспозицию (тот же finite()) |
| 39 | `luxel/shaders/grab_down.frag:28` | Firefly suppression turns an Inf texel into NaN across the transmission mip chain | подтверждена | исправлена 2b9fca4 | Inf в grab_down → NaN в мипах пропускания |
| 40 | `luxel/shaders/gtao.frag:91` | normal_from_depth unprojects sky neighbours to Inf/NaN under reverse-Z or an infinite far plane | подтверждена | исправлена 879f92f | GTAO: небо при reverse-Z даёт NaN нормаль |
| 41 | `luxel/shaders/lib/surface_openpbr.glsl:293` | Back-lit terms (punctual transmission, diffuse transmission, subsurface wrap) are cancelled by the mesh's own  | подтверждена | исправлена 879f92f | собственная тень гасит просвет (transmission/subsurface) |
| 42 | `luxel/shaders/pbr.vert:37` | `flat` varying v_model_scale in every PBR variant forces ANGLE's flat-shading emulation on all PBR draws | подтверждена | исправлена 4d8ad29 | flat varying во всех PBR-вариантах |
| 43 | `luxel/shaders/taa.frag:20` | Late (unjittered) meshes are depth-tested against the jittered opaque depth: 1px crawl on every occluding silh | подтверждена | задокументирована 4d8ad29 | 1px мерцание поздних мешей с TAA — следствие дизайна; задокументировать |
| 44 | `luxel/shaders/taa.frag:139` | After a TAA reset the single aliased frame keeps ~64% weight for the 30 'settle' frames | подтверждена | исправлена 0410b8f | после сброса TAA алиасный кадр держит ~64% веса |
| 45 | `luxel/src/environment.rs:356` | BakeJob::start never checks the image against MAX_TEXTURE_SIZE (or zero size); the bake fails later with a wro | подтверждена | исправлена c7f2e4c | HDR больше MAX_TEXTURE_SIZE: ошибка с неверной причиной |
| 46 | `luxel/src/environment.rs:411` | Bake allocates the full-resolution equirect as RGBA16F with a full mip chain regardless of the cube size | подтверждена | исправлена c7f2e4c | запекание держит полный equirect 16F с мипами |
| 47 | `luxel/src/exposure.rs:169` | dt <= 0 is treated as 'no history' and snaps the exposure; Date.now's coarse, non-monotonic clock makes that h | подтверждена | исправлена 2b9fca4 | dt<=0 сбрасывает экспозицию |
| 48 | `luxel/src/exposure.rs:174` | Auto exposure easing factor speed*dt is not clamped to 1: overshoots and oscillates between min and max | подтверждена | исправлена 2b9fca4 | speed*dt не ограничен 1: осцилляция |
| 49 | `luxel/src/exposure.rs:177` | One Inf/NaN texel in the HDR frame poisons the metered exposure (black screen for many seconds, or for good wi | дубль 38 | исправлена 2b9fca4 |  |
| 50 | `luxel/src/gpu_pick.rs:120` | GPU pick draws back faces the scene culls, so invisible surfaces are picked | подтверждена | исправлена 02b6a17 | GPU-пикинг не отсекает задние грани |
| 51 | `luxel/src/hierarchy.rs:215` | A hierarchy rebuild restamps every WorldMatrix as changed, invalidating all shadow-map caches and object group | подтверждена | исправлена 257b1dc | перестройка иерархии штампует все WorldMatrix |
| 52 | `luxel/src/js/capture.rs:79` | A capture too big for the GPU permanently turns off TAA / HDR / MSAA for the live canvas | неясно | исправлена иначе | по опыту нехватка памяти теряет контекст: capture теперь отклоняется ContextLostError и возвращает размер канваса |
| 53 | `luxel/src/js/lights.rs:421` | setCompactEnvironment during a first bake leaves the new environment at the old intensity (0.2) | подтверждена | исправлена b79e023 | setCompactEnvironment во время первого запекания: интенсивность 0.2 |
| 54 | `luxel/src/js/materials.rs:366` | loadTexture promise rejects with "can't load" when the context is lost mid-load, although the scene reloads th | подтверждена | исправлена 02b6a17 | loadTexture при потере контекста отклоняется, хотя текстура перезагружается |
| 55 | `luxel/src/js/materials.rs:366` | loadTexture in flight at a loss rejects with "can't load" while the scene keeps and reloads the texture; faile | дубль 54 | исправлена 02b6a17 |  |
| 56 | `luxel/src/js/materials.rs:366` | A loadTexture in flight at a context loss rejects with 'can't load', yet the scene reloads the texture under a | дубль 54 | исправлена 02b6a17 |  |
| 57 | `luxel/src/js/materials.rs:366` | loadTexture in flight at a context loss rejects with 'can't load', while the scene reloads the texture under a | дубль 54 | исправлена 02b6a17 |  |
| 58 | `luxel/src/js/materials.rs:381` | A loadTexture that fails stays in scene.textures forever and is retried after every context loss | подтверждена | исправлена 02b6a17 | неудачный loadTexture остаётся и перезагружается после потери |
| 59 | `luxel/src/js/materials.rs:459` | setMaterialTexture stores texCoord as a shader UV slot, not as a TEXCOORD set: wrong UVs on materials whose sl | подтверждена | исправлена 6bc5bfc | setMaterialTexture: texCoord как слот, не как UV-набор |
| 60 | `luxel/src/js/mod.rs:361` | setLocalMatrix with a zero scale on any axis (or non-finite input) stores a NaN rotation that never recovers | подтверждена | исправлена 1171b2a |  |
| 61 | `luxel/src/js/mod.rs:368` | lookAt, setLocalMatrix and setBounds skip the finite check every other transform setter applies | подтверждена | исправлена e7928f3 | lookAt/setLocalMatrix/setBounds без проверки на конечность |
| 62 | `luxel/src/js/mod.rs:595` | createProgram records are never dropped: destroyed shaders are recompiled after every loss, and the recompiled | подтверждена | исправлена 6bc5bfc | createProgram: исходники навсегда, перекомпиляция уничтоженных |
| 63 | `luxel/src/js/mod.rs:595` | createProgram sources are kept forever and destroyed programs are recompiled after a context loss | дубль 62 | исправлена 6bc5bfc |  |
| 64 | `luxel/src/js/mod.rs:618` | addModel / instantiate while the context is lost spawn a model without primitives, for good | подтверждена | исправлена 02b6a17 | addModel при потерянном контексте: модель без примитивов навсегда |
| 65 | `luxel/src/js/mod.rs:618` | addModel / instantiate while the context is lost take the Model over but spawn none of its primitives, for goo | дубль 64 | исправлена 02b6a17 |  |
| 66 | `luxel/src/js/mod.rs:674` | instantiate()/addModel() while the context is lost spawn a model with no primitives, permanently | дубль 64 | исправлена 02b6a17 |  |
| 67 | `luxel/src/js/mod.rs:951` | Scene.render() into a user pass corrupts TAA motion-vector history of renderFrame | подтверждена | исправлена e7928f3 | Scene.render() портит историю движения TAA |
| 68 | `luxel/src/js/mod.rs:956` | Scene.render() consumes the render-on-demand state, so the canvas frame is skipped (or never idles) | подтверждена | исправлена e7928f3 | Scene.render() съедает состояние render-on-demand |
| 69 | `luxel/src/js/mod.rs:1141` | Depth peeling runs the full PBR fragment shader once per layer into depth-only targets | подтверждена | исправлена cf793ab | depth peeling гоняет полный PBR-шейдер |
| 70 | `luxel/src/js/on_demand.rs:106` | With TAA, content that lands inside a frame gets no settle frames: needsRender goes false on an unconverged im | подтверждена | исправлена e7928f3 | TAA + on-demand: контент внутри кадра без settle-кадров |
| 71 | `luxel/src/js/on_demand.rs:109` | A frame attempted while the context is lost consumes the 'generation changed' signal; needsRender never trigge | подтверждена | исправлена e7928f3 | кадр при потерянном контексте съедает сигнал generation |
| 72 | `luxel/src/js/on_demand.rs:110` | With TAA and render-on-demand, async work applied inside a frame gets no settle frames | дубль 70 | исправлена e7928f3 |  |
| 73 | `luxel/src/js/openpbr.rs:102` | fuzz_weight / fuzz_color do not round-trip: weight is re-derived as max(sheenColor) | подтверждена | исправлена c50342c |  |
| 74 | `luxel/src/js/picking.rs:111` | screenToRay / raycastBounds are broken for cameras with an infinite far plane | подтверждена | исправлена d2c4abe |  |
| 75 | `luxel/src/js/restore.rs:105` | Loss during an environment bake restores the environment at the pre-bake intensity instead of 1 | подтверждена | исправлена b79e023 | потеря во время запекания: интенсивность до запекания |
| 76 | `luxel/src/js/restore.rs:105` | A loss while an environment bake is pending restores the environment at the pre-bake intensity instead of 1 | дубль 75 | исправлена b79e023 |  |
| 77 | `luxel/src/js/restore.rs:141` | scene_programs keeps and recompiles shaders the app already destroyed; alias maps never shrink | дубль 62 | исправлена 6bc5bfc |  |
| 78 | `luxel/src/js/restore.rs:148` | A loss in the middle of Scene::restore loses shaders permanently: scene programs are dropped, PBR meshes stop  | подтверждена | исправлена 6bc5bfc | потеря посреди restore теряет шейдеры |
| 79 | `luxel/src/js/restore.rs:216` | loadTexture in flight at a loss rejects with 'can't load', yet the scene reloads the texture under a handle no | дубль 54 | исправлена 02b6a17 |  |
| 80 | `luxel/src/js/restore.rs:308` | App meshes parented under a model get Reloading and never lose it | подтверждена | исправлена b79e023 | меши приложения под моделью навсегда Reloading |
| 81 | `luxel/src/js/restore.rs:308` | App meshes parented under a model are marked Reloading at a restore and never unmarked | дубль 80 | исправлена b79e023 |  |
| 82 | `luxel/src/js/restore.rs:308` | App meshes parented under a model are marked Reloading after a loss and never unmarked | дубль 80 | исправлена b79e023 |  |
| 83 | `luxel/src/js/restore.rs:346` | A failed model reload leaves the model invisible for good, with needsRender stuck true and no signal to the ap | подтверждена | исправлена 4fd9e30 | whenReady отклоняется при неудачной перезагрузке |
| 84 | `luxel/src/js/restore.rs:346` | A failed (or disabled) model reload leaves the meshes Reloading forever: needsRender stuck true, whenReady res | дубль 83 | исправлена b79e023 |  |
| 85 | `luxel/src/lighting/maps.rs:152` | The shadow array is never shrunk back once no light casts shadows | подтверждена | исправлена 257b1dc | массив теней не сжимается |
| 86 | `luxel/src/lighting/mod.rs:584` | Shadow map array is never shrunk or freed once no light casts shadows | дубль 85 | исправлена 257b1dc |  |
| 87 | `luxel/src/lighting/pack.rs:97` | Lights past MAX_LIGHTS still get shadow maps planned and drawn, and can be picked as the key light | подтверждена | исправлена 4d8ad29 | свет сверх MAX_LIGHTS всё равно получает тени |
| 88 | `luxel/src/model.rs:85` | Node TRS is re-derived from the matrix, so mirrored nodes animate wrongly when a clip targets only rotation or | подтверждена | исправлена 11e45e2 | TRS из матрицы: зеркальные узлы анимируются неверно |
| 89 | `luxel/src/model.rs:178` | spawn_model deep-copies all animation keyframes for every spawn and every instantiate | подтверждена | исправлена 4d8ad29 | копия ключей анимаций на каждый spawn |
| 90 | `luxel/src/motion.rs:144` | Motion-vector history is per renderer, so any extra render() between renderFrames wipes the moved-mesh detecti | дубль 67 | исправлена e7928f3 |  |
| 91 | `luxel/src/output.rs:685` | The canvas clear color goes through exposure, auto exposure and tone mapping in HDR, so it is not shown as set | подтверждена | задокументирована 4d8ad29 | clear color в HDR проходит экспозицию и тонмаппинг |
| 92 | `luxel/src/output.rs:789` | Transparent meshes vanish behind glass with MSAA and a scaled transmission copy (the "medium" preset) | подтверждена | исправлена 0e6cf6f | прозрачные меши пропадают за стеклом при MSAA+масштабе |
| 93 | `luxel/src/output.rs:797` | An incomplete grab framebuffer keeps its depth renderbuffer attached, breaking the whole transmission copy | подтверждена | исправлена 4d8ad29 | неполный grab FBO ломает всю копию |
| 94 | `luxel/src/output.rs:797` | An incomplete grab framebuffer keeps its depth renderbuffer, so every later scene copy blit fails | дубль 93 | исправлена 4d8ad29 |  |
| 95 | `luxel/src/render.rs:773` | A second render call in the same frame wipes the motion-vector history, so moving meshes ghost under TAA | дубль 67 | исправлена e7928f3 |  |
| 96 | `luxel/src/render.rs:824` | Meshes left Reloading for good count as pending every frame: needsRender never goes false | подтверждена | исправлена b79e023 | Reloading навсегда считается pending |
