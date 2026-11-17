// Compile-time checks of the npm package's typings: what should type-check
// does, and each `@ts-expect-error` line must fail to. Not run, only checked:
//
//   node scripts/build-npm.mjs && deno check scripts/typings-check.ts
/// <reference lib="dom" />
// @ts-types="../dist/npm/rswebgl.d.ts"
import init, {
  Context,
  type CameraOptions,
  type AnimationEvent,
  type BackgroundFill,
  type Entity,
  type Handles,
  type LightOptions,
  loadModel,
  type Model,
  type ParamGroupId,
  type ProgramId,
  Scene,
  type TextureId,
} from "../dist/npm/rswebgl.js";

export async function check(canvas: HTMLCanvasElement, ctx: Context, scene: Scene) {
  await init();
  const model: Model = await loadModel(ctx, "a.glb");
  const root: Entity = scene.addModel(model);
  const dropped = await loadModel(ctx, new File([], "a.gltf"), {
    files: [new File([], "a.bin")],
    signal: new AbortController().signal,
    onProgress: (loaded, total) => loaded / total,
  });
  await dropped.whenLoaded();
  const warnings: string[] = dropped.warnings();
  // @ts-expect-error not an option
  await loadModel(ctx, "a.glb", { vertexCompresion: true });
  const e: Entity = scene.spawn();
  scene.setParent(e, root);
  const tex: TextureId = await scene.loadTexture("a.png");
  const mats: Handles<ParamGroupId> = scene.materials(root);
  const m: ParamGroupId = mats[0];
  scene.setMaterialTexture(m, "baseColor", tex, undefined);
  for (const cam of scene.cameras(root)) scene.renderFrame(cam as Entity);
  const ready: void = await scene.whenReady();
  scene.setToneMapping("agx");
  const name: "openpbr" | "gltf" = scene.shadingModel();
  const hit = await scene.pick(e, 0, 0);
  if (hit) scene.setVisible(hit.entity, false);
  const p: ProgramId = ctx.createProgram("", "");
  await ctx.programsReady([p]);
  ctx.setOnLost(() => {});
  ctx.textureLoadImage(ctx.createTexture(0 as never, 0 as never, 0 as never), "a.png", undefined!, true, false, (w, h) => w + h);
  const bytes: Uint8Array = await ctx.readPixelsAsync(0 as never, 0, 0, 0, 1, 1, 0);

  // @ts-expect-error a texture isn't an entity
  scene.setVisible(tex, false);
  // @ts-expect-error a plain number isn't a handle
  scene.despawn(5);
  // @ts-expect-error an entity isn't a material
  scene.setMaterialTexture(e, "baseColor", tex, undefined);
  // @ts-expect-error not a tone mapping
  scene.setToneMapping("filmic");
  // @ts-expect-error a material isn't a program
  ctx.destroyProgram(m);
  return [canvas, ready, name, bytes, warnings];
}

// Getters read back what setters take.
declare const gs: Scene;
declare const ge: Entity;
const camOpts: CameraOptions | undefined = gs.camera(ge);
if (camOpts) gs.setCamera(ge, camOpts);
const lightOpts: LightOptions | undefined = gs.light(ge);
if (lightOpts) gs.setLight(ge, lightOpts);
const kids: Handles<Entity> = gs.children(ge);
const firstKid: Entity = kids[0];
const variantName: string | undefined = gs.materialVariant(ge);
const blur: number | undefined = gs.background();
void [firstKid, variantName, blur];

// Animation events name their model root and clip.
const animEvents: AnimationEvent[] = gs.advanceAnimations(0.016);
for (const ev of animEvents) {
  const evRoot: Entity = ev.root;
  if (ev.type === "finished") gs.crossfadeAnimation(evRoot, ev.animation, 0.3, true);
  // @ts-expect-error: only "finished" and "loop"
  if (ev.type === "started") void 0;
}
gs.setAnimationTime(ge, 0.5);

// Background colors and gradients.
gs.setBackgroundColor([1, 1, 1]);
gs.setBackgroundGradient([1, 1, 1], new Float32Array([0.8, 0.8, 0.8]), "radial");
// @ts-expect-error: "vertical" or "radial" only
gs.setBackgroundGradient([1, 1, 1], [0, 0, 0], "conic");
const fill: BackgroundFill | undefined = gs.backgroundFill();
if (fill?.type === "vertical") void fill.to;

// Scene loss events.
gs.setOnLost(() => console.log("lost"));
gs.setOnRestored(undefined);
