# luxel

A WebGL2 renderer written in Rust, compiled to WebAssembly. It has two levels:

- **`Scene`**: a scene graph with glTF 2.0 models (compressed ones too),
  OpenPBR or glTF-reference shading, image-based light from `.hdr`
  environments, HDR output with tone mapping, MSAA or temporal
  antialiasing, picking, instancing, skinning, morph targets, animation and
  material variants.
- **The low-level API** under it (`Context`, `Pass`, programs, buffers,
  textures, render states, queries) for drawing your own way.

```js
import init, { Context, ContextOptions, Scene, loadModel } from "@fairlybet/luxel";

await init(); // fetches luxel_bg.wasm next to the module

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

`@fairlybet/luxel/orbit` ships an orbit camera (drag, wheel / pinch, right-drag to
pan, a tap for picking). `scene.frameBounds(camera, root)` points a camera
at a model. `scene.needsRender()` says whether anything changed since the
last frame, so a still scene costs nothing:

```js
import { OrbitControls } from "@fairlybet/luxel/orbit";

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

## Animation

A model's glTF animations are listed by `scene.animations(root)` and play
on its root. `advanceAnimations(seconds)` moves every playing one on and
returns what happened on the way: `{ type: "finished" | "loop", root,
animation }`.

```js
scene.playAnimation(root, 0, true);              // index, looping
scene.crossfadeAnimation(root, 1, 0.4, false);   // fade into clip 1 over 0.4 s
scene.setAnimationTime(root, 1.25);              // seek
scene.setAnimationSpeed(root, -1);               // backwards
for (const e of scene.advanceAnimations(dt)) {
  if (e.type === "finished") scene.crossfadeAnimation(e.root, 0, 0.4, true);
}
```

During a crossfade both clips move on and their poses mix by a weight
rising from 0 to 1. A property only one clip animates mixes with where it
stood when the fade began.

## Background

Behind the scene goes one of:
- the environment: `scene.setBackground(blur)`, with blur from 0 (sharp)
  to 1;
- a flat color: `scene.setBackgroundColor([1, 1, 1])`;
- a gradient: `scene.setBackgroundGradient(from, to, "vertical" | "radial")`.

Colors are display values (sRGB 0–1, like CSS). They show exactly as given,
neither exposed nor tone mapped, so white stays white. Glass and other
transmissive materials see them behind the scene.

`scene.clearBackground()` removes all of these: the canvas's clear color
shows, or the page behind a transparent canvas.

## Quality, resolution, screenshots

`scene.setQuality("low" | "medium" | "high")` sets antialiasing, shadow
map size, ambient occlusion (on in "high"), transmission resolution and a
pixel-ratio cap together; each can still be set after. Contact shadows stay
off in every preset (`setContactShadows` turns them on).

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
import { setDracoDecoderPath, setKtx2TranscoderPath } from "@fairlybet/luxel";

setDracoDecoderPath("https://www.gstatic.com/draco/versioned/decoders/1.5.7/");
setKtx2TranscoderPath("https://cdn.jsdelivr.net/npm/three@0.170.0/examples/jsm/libs/basis/");
```

## Antialiasing

The scene's HDR target is multisampled by default (`scene.setSamples(4)`).
`scene.setTaa(true)` switches to temporal antialiasing, which also smooths
shading and thin geometry; pair it with `scene.setSharpening(0.5)`.
`scene.setFxaa(true)` is the cheap one for phones (with `setSamples(0)`);
with TAA it covers what TAA leaves out (glass and transparent meshes).

## Shadows and occlusion

- Lights cast shadow maps with `castShadow: true` in `setLight`:
  directional (with cascades), spot and point lights;
  `scene.setShadowMapSize(2048)` sets their resolution.
- `scene.setAmbientOcclusion({})` darkens creases and contacts (GTAO);
  options `radius`, `intensity`, …, or `null` to turn it off.
- `scene.setGroundShadow({})`: a soft shadow on an invisible ground under
  the model, for scenes lit by their environment alone.
- `scene.setContactShadows({})`: short screen-space shadows where things
  touch, of the light giving the most light at the scene's centre. Off by
  default: they show only the start of a long shadow and change with the
  view.

Each takes its options or `null`, and its getter
(`ambientOcclusion()`, `groundShadow()`, `contactShadows()`) gives them
back (defaults filled in) or `null`.

## Context loss

When the browser takes the WebGL context away (a GPU reset, too many tabs),
the scene rebuilds itself on its first call after the context is back:
- shaders, the environment and `loadTexture` textures are made again;
- models are fetched again from their URLs (`setRestoreModels(false)` turns
  that off).

Material handles and edits survive. `scene.whenReady()` resolves once
everything is back, models with their images; it rejects while the context
is lost. `scene.setOnLost(f)` and `scene.setOnRestored(f)` tell the app.
Rebuild what you made yourself (meshes from `setMesh`, programs from
`ctx`) in `setOnRestored`:

```js
scene.setOnLost(() => overlay.show());
scene.setOnRestored(async () => {
  rebuildMyMeshes();
  await scene.whenReady();
  overlay.hide();
});
```

## Bundlers

The module finds its `.wasm` with `new URL("luxel_bg.wasm", import.meta.url)`,
which Vite, webpack 5 and Parcel follow. esbuild doesn't: import the file
and pass its URL (`--loader:.wasm=file`, with `--public-path` set to where
the bundle is served from):

```js
import wasmUrl from "@fairlybet/luxel/luxel_bg.wasm";
await init({ module_or_path: wasmUrl });
```

Without a bundler, serve the package's files and import `luxel.js`; serve
`.wasm` as `application/wasm`.

## Types, handles and memory

Types are in `luxel.d.ts`. Entities, materials, textures, programs and the
other GPU objects are **handles**: plain numbers at run time, typed by kind
(`Entity`, `ParamGroupId`, `TextureId`, `ProgramId`, …), so TypeScript
catches a texture passed where an entity goes. Handles are freed explicitly
(`scene.despawn`, `scene.destroyTexture`, `ctx.destroyProgram`, …).
`cameras`, `lights`, `materials` and `children` return `Handles<T>`: a
`Uint32Array` whose elements read as `T`.

A setting's getter has its setter's name without `set`, and reads the
value back in the shape the setter takes:
- `camera(e)` and `light(e)` give the options of `setCamera` and
  `setLight`;
- `ambientOcclusion()`, `contactShadows()` and `groundShadow()` give their
  options, or `null` when off;
- flags: `hdr()`, `taa()`, `fxaa()`, `autoExposure()`, `reverseZ()`,
  `dither()`, `restoreModels()`, `gpuProfiling()`;
- also `exposure()`, `environmentIntensity()`, `backgroundFill()`,
  `cameraAspect(e)`, `visible(e)`, `localMatrix(e)`, `bounds(e)`,
  `animationSpeed(root)` and `materialVariant(root)`.

`isX` / `hasX` ask about a state rather than a setting: `isAlive(e)`,
`isShown(e)`, `hasEnvironment()` (and `ctx.isLost()` on the context).

A setter's error names it: `"setLight: \"intensity\" must be a finite
number"`.

`visible(e)` is `e`'s own flag. `isShown(e)` also counts hidden ancestors,
as glTF's `KHR_node_visibility` does.

`scene.destroy()` frees everything the scene owns (its models, targets,
shadow maps, environment, cloned materials, loaded textures), and
`model.destroy(ctx)` a model you didn't add to a scene. A `Scene` or
`Model` you only drop is freed the same way by `free()`, or by the
garbage collector once unreachable — but when it gets round to it:
destroy large ones yourself so the GPU memory comes back now.

The other objects (`Pass`, `FrameStats`, `RenderState`, …) are freed by
the garbage collector, or at once by `free()`.

## License

MIT. Third-party code it ports (MaterialX, Khronos, three.js, Filament,
others) is listed in `THIRD_PARTY.md`.
