# rswebgl

A WebGL2 renderer written in Rust, compiled to WebAssembly. It has two levels:

- **`Scene`**: a scene graph with glTF 2.0 models (compressed ones too),
  OpenPBR or glTF-reference shading, image-based light from `.hdr`
  environments, HDR output with tone mapping, MSAA or temporal
  antialiasing, picking, instancing, skinning, morph targets, animation and
  material variants.
- **The low-level API** under it (`Context`, `Pass`, programs, buffers,
  textures, render states, queries) for drawing your own way.

```js
import init, { Context, ContextOptions, Scene, loadModel } from "rswebgl";

await init(); // fetches rswebgl_bg.wasm next to the module

const canvas = document.querySelector("canvas");
const ctx = Context.fromCanvas(canvas, new ContextOptions());
ctx.defaultFramebuffer().enableAutoResize();
const scene = new Scene(ctx);

const root = scene.addModel(await loadModel(ctx, "model.glb"));
const [cx, cy, cz, r] = scene.modelBounds(root);
const camera = scene.spawn();
scene.setCamera(camera, { type: "perspective", yfov: 0.8, znear: r * 0.01, zfar: r * 100 });
scene.lookAt(camera, [cx, cy, cz + r * 2.5], [cx, cy, cz]);

const hdr = new Uint8Array(await (await fetch("studio.hdr")).arrayBuffer());
scene.setEnvironmentHdr(hdr, 256);
await scene.whenReady();

let last = performance.now();
requestAnimationFrame(function frame(now) {
  scene.advanceAnimations((now - last) / 1000);
  last = now;
  scene.renderFrame(camera);
  requestAnimationFrame(frame);
});
```

## Loading models

`loadModel(ctx, source, options?)` takes a URL, a `File` / `Blob` or the
file's bytes. It resolves once the geometry is in; the images arrive after
that. `model.whenLoaded()` resolves when they are all in or have failed
(`scene.whenReady()` waits for them too), and `model.warnings()` says what
failed.

```js
const abort = new AbortController();
const model = await loadModel(ctx, "https://cdn.example.com/chair.glb", {
  signal: abort.signal,                       // cancels every request
  request: { credentials: "include" },        // headers, credentials, …
  resolveUrl: (url) => sign(url),             // rewrite each URL
  onProgress: (loaded, total) => bar(loaded, total),
});
await model.whenLoaded();
console.log(model.warnings());
```

A `.gltf` dropped or picked with its `.bin` and textures loads from the
files themselves:

```js
const files = [...event.dataTransfer.files];
const main = files.find((f) => /\.(glb|gltf)$/i.test(f.name));
const model = await loadModel(ctx, main, { files });
```

Texture and vertex compression can be set per load with
`textureCompression` and `vertexCompression`; by default each follows its
module-wide switch. A file whose POSITION accessors lack `min` / `max` still
loads, with a warning; other validation errors reject the load.

## Camera, render on demand, hotspots

`rswebgl/orbit` ships an orbit camera (drag, wheel / pinch, right-drag to
pan, a tap for picking). `scene.frameBounds(camera, root)` points a camera
at a model. `scene.needsRender()` says whether anything changed since the
last frame, so a still scene costs nothing:

```js
import { OrbitControls } from "rswebgl/orbit";

const orbit = new OrbitControls(scene, camera, canvas);
orbit.frame(root);
orbit.onTap = (x, y) => scene.pick(camera, x, y).then((hit) => console.log(hit));
requestAnimationFrame(function frame(now) {
  orbit.update(1 / 60);
  scene.advanceAnimations(1 / 60);
  if (scene.needsRender(camera) || !scene.presentFrame()) scene.renderFrame(camera);
  requestAnimationFrame(frame);
});
```

Call `scene.invalidate()` for a change the scene can't see (a texture you
filled yourself). For hotspots and annotations:
- `scene.worldToScreen(camera, point)` gives `[x, y, depth]`: normalized
  device coordinates, and the depth, which is ≤ 0 behind the camera.
- `scene.screenToRay(camera, x, y)` gives a world-space ray.
- `scene.raycastBounds(camera, x, y)` is a synchronous hit test against the
  meshes' boxes; `pick` is the exact test on the GPU.

## The same model twice

`scene.instantiate(root)` spawns a model added with `addModel` again, under
a new root, sharing its vertex arrays, textures and materials: no second
load, no second copy on the GPU. Each copy has its own transforms,
visibility, animation and material variant; a material edited shows on
every copy (`cloneMaterial` + `setMaterial` for one of its own).

## Quality, resolution, screenshots

`scene.setQuality("low" | "medium" | "high")` sets antialiasing, shadow
map size, ambient occlusion and contact shadows, transmission resolution and
a pixel-ratio cap together; each can still be set after.

With auto-resize on, the canvas's drawing buffer can be scaled:
- `ctx.defaultFramebuffer().setResolutionScale(0.75)` draws fewer pixels;
- `setMaxPixelRatio(2)` caps the pixels per CSS pixel on dense screens.

`await scene.capture(camera, { width: 2048, height: 2048 })` gives a PNG
`Blob` drawn as frames are (`type: "image/jpeg"`, `quality` for others).

## Compressed glTF

meshopt-compressed files load as they are. Draco meshes and KTX2 (Basis
Universal) textures are decoded in workers by decoders you point the loader
at, before loading such a model (three.js' conventions: a directory with
`draco_wasm_wrapper.js` + `draco_decoder.wasm`, and one with
`basis_transcoder.js` + `basis_transcoder.wasm`):

```js
import { setDracoDecoderPath, setKtx2TranscoderPath } from "rswebgl";

setDracoDecoderPath("https://www.gstatic.com/draco/versioned/decoders/1.5.7/");
setKtx2TranscoderPath("https://cdn.jsdelivr.net/npm/three.170.0/examples/jsm/libs/basis/");
```

## Antialiasing

The scene's HDR target is multisampled by default (`scene.setSamples(4)`).
`scene.setTaa(true)` switches to temporal antialiasing, which also smooths
shading and thin geometry; pair it with `scene.setSharpening(0.5)`.

## Bundlers

The module finds its `.wasm` with `new URL("rswebgl_bg.wasm", import.meta.url)`,
which Vite, webpack 5 and Parcel follow. esbuild doesn't: import the file
and pass its URL (`--loader:.wasm=file`, with `--public-path` set to where
the bundle is served from):

```js
import wasmUrl from "rswebgl/rswebgl_bg.wasm";
await init({ module_or_path: wasmUrl });
```

Without a bundler, serve the package's files and import `rswebgl.js`; serve
`.wasm` as `application/wasm`.

## Types, handles and memory

Types are in `rswebgl.d.ts`. Entities, materials, textures, programs and the
other GPU objects are **handles**: plain numbers at run time, typed by kind
(`Entity`, `ParamGroupId`, `TextureId`, `ProgramId`, …), so TypeScript
catches a texture passed where an entity goes. Handles are freed explicitly
(`scene.despawn`, `scene.destroyTexture`, `ctx.destroyProgram`, …).
`cameras`, `lights` and `materials` return `Handles<T>`: a `Uint32Array`
whose elements read as `T`.

Objects (`Scene`, `Model`, `Pass`, `FrameStats`, `RenderState`, …) are
freed by the garbage collector once unreachable; `free()` frees one at
once, which matters for large ones such as a `Model` or a `Scene`.

## License

MIT. Third-party code it ports (MaterialX, Khronos, three.js, Filament,
others) is listed in `THIRD_PARTY.md`.
