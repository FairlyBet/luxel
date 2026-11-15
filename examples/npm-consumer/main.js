// The npm package through a bundler: `import … from "rswebgl"`. esbuild
// doesn't follow the module's `new URL("rswebgl_bg.wasm", import.meta.url)`,
// so the .wasm is imported here (its file loader copies it and gives its
// URL) and handed to init. Vite and webpack 5 need neither.
import init, { Context, ContextOptions, Scene, loadModel } from "rswebgl";
import { OrbitControls } from "rswebgl/orbit";
import wasmUrl from "rswebgl/rswebgl_bg.wasm";

const status = /** @type {HTMLElement} */ (document.getElementById("status"));
await init({ module_or_path: wasmUrl });
const canvas = /** @type {HTMLCanvasElement} */ (document.getElementById("c"));
const ctx = Context.fromCanvas(canvas, new ContextOptions());
ctx.defaultFramebuffer().setClearColor(0.1, 0.1, 0.12, 1);
ctx.defaultFramebuffer().enableAutoResize();
const scene = new Scene(ctx);

const url =
  new URLSearchParams(location.search).get("model") ??
  "https://raw.githubusercontent.com/KhronosGroup/glTF-Sample-Assets/main/Models/DamagedHelmet/glTF-Binary/DamagedHelmet.glb";
const model = await loadModel(ctx, url, {
  onProgress: (loaded) => (status.textContent = `loading… ${(loaded / 1048576).toFixed(1)} MB`),
});
const root = scene.addModel(model);
const camera = scene.spawn();
scene.setCamera(camera, { type: "perspective", yfov: 0.8, znear: 0.01, zfar: 100 });
const light = scene.spawn();
scene.setLight(light, { type: "directional", intensity: 3 });
scene.lookAt(light, [1, 2, 3], [0, 0, 0]);

// Drag, wheel / pinch, right-drag to pan; a tap names what's under it.
const orbit = new OrbitControls(scene, camera, canvas);
orbit.yaw = 0.6;
orbit.frame(root);
orbit.onTap = async (x, y) => {
  const hit = await scene.pick(camera, x, y);
  status.textContent = hit ? `picked ${scene.name(scene.parent(hit.entity)) ?? hit.entity}` : "nothing there";
};
await scene.whenReady();
status.textContent = "drag to orbit";

// Drawn only when something changed; the last frame shown again otherwise.
let last = performance.now();
let frames = 0;
requestAnimationFrame(function frame(now) {
  orbit.update((now - last) / 1000);
  last = now;
  if (scene.needsRender() || !scene.presentFrame()) {
    scene.renderFrame(camera);
    frames++;
  }
  requestAnimationFrame(frame);
});
Object.assign(globalThis, { scene, orbit, framesDrawn: () => frames });
