//! Option objects from JS: plain objects read by key, with TypeScript types
//! for them in the generated `.d.ts`. A malformed object is an error the call
//! throws — options are setup, not per-frame data.

use glam::Vec3;
use js_sys::Reflect;
use luxel_gl::error::Error;
use wasm_bindgen::prelude::*;

#[wasm_bindgen(typescript_custom_section)]
const TS_OPTIONS: &str = r#"
/** An entity: a plain number (the npm package's typings brand it). */
export type Entity = number;
/** The background color or gradient (`setBackgroundColor`,
 *  `setBackgroundGradient`), in display (sRGB 0–1) values. */
export interface BackgroundFill {
  type: "color" | "vertical" | "radial";
  /** The color; a gradient's bottom ("vertical") or center ("radial"). */
  from: Float32Array;
  /** A gradient's top or corners. */
  to?: Float32Array;
}

/** A punctual light (names and units as in KHR_lights_punctual). */
export interface LightOptions {
  type: "directional" | "point" | "spot";
  /** Linear RGB; default white. */
  color?: ArrayLike<number>;
  /** Lux for directional lights, candela otherwise; default 1. */
  intensity?: number;
  /** Point / spot cutoff distance; 0 or absent = none. */
  range?: number;
  /** Spot cone, radians from the axis; defaults 0 and π/4. */
  innerConeAngle?: number;
  outerConeAngle?: number;
  /** Cast shadows (`setShadowMapSize`): a map for a spot light, one per
   *  cascade for a directional light, six (a quarter layer each) for a
   *  point light; default false. */
  castShadow?: boolean;
  /** Shadow biases (Filament's defaults): receivers moved along their
   *  normal by this many shadow-map texels (1); casters pushed back by
   *  this × their slope (2) + this (0.5) depth units. Raise for acne
   *  (stripes on lit surfaces), lower for detached shadows. */
  shadowNormalBias?: number;
  shadowSlopeBias?: number;
  shadowConstantBias?: number;
  /** Directional: the most cascades its shadow splits into along the view
   *  (1–4; default 3). Only where the view sees more than a model: one in
   *  view keeps one map fitted to it. */
  shadowCascades?: number;
  /** Directional: how the cascades split, from uniform slices (0) to
   *  logarithmic ones (1, more texels near the eye); default 0.5. */
  shadowSplitLambda?: number;
  /** Directional: no shadows farther along the view (0, the default: as
   *  far as the casters reach). */
  shadowMaxDistance?: number;
  /** The side of this light's shadow maps in texels (a point light's
   *  faces): a power of two up to the scene's `setShadowMapSize`, packed
   *  with the other lights' maps into its layers. Default (0): the scene's
   *  size, half of it for a point light's faces. A small spot light can
   *  take 512 and leave the rest of a layer to others. */
  shadowMapSize?: number;
}

/** The ground shadow (`setGroundShadow`): a soft dark patch from the
 *  shadow casters, on the ground under every visible mesh (glass too) at
 *  their lowest point. */
export interface GroundShadowOptions {
  /** How dark where a caster touches the ground; default 1 (above 1
   *  darkens higher parts too). */
  darkness?: number;
  /** The blur's radius, in 256ths of the patch; default 3.5. */
  blur?: number;
  /** The patch's opacity, 0..1; default 1. */
  opacity?: number;
  /** How far above the ground a caster still darkens it, in world units;
   *  default 0: half the casters' height. */
  reach?: number;
  /** The darkness texture's side in texels: rounded up to a power of two
   *  in 64..2048 (what `groundShadow()` reports); default 512. */
  size?: number;
}

/** Ambient occlusion (`setAmbientOcclusion`, GTAO). Lengths in world units. */
export interface AmbientOcclusionOptions {
  /** How far around a point occluders count; default 0: a twentieth of
   *  the scene's size. */
  radius?: number;
  /** How far in front of a point an occluder may be and still count;
   *  default 0: four radii. */
  thickness?: number;
  /** How much of the darkening shows, 0..1; default 1. */
  intensity?: number;
  /** The visibility raised to this (above 1: darker creases); default 1. */
  power?: number;
  /** Computed at half resolution (cheaper, softer); default true. */
  halfResolution?: boolean;
}

/** Contact shadows (`setContactShadows`) of the key light: the one giving
 *  the most light at the scene's centre (directional, point or spot).
 *  Lengths in world units. */
export interface ContactShadowsOptions {
  /** How far toward the light to look; default 0: a fiftieth of the
   *  scene's size. */
  length?: number;
  /** How deep a surface in front may be and still shadow; default 0: a
   *  quarter of the length. */
  thickness?: number;
  /** How much of the shadow shows, 0..1; default 1. */
  intensity?: number;
  /** Steps along the ray, 1..64; default 16. */
  steps?: number;
}

/** A camera. It looks down its entity's −Z, +Y up. */
export type CameraOptions =
  | { type: "perspective"; /** radians; default π/4 */ yfov?: number; znear?: number; /** Infinity: no far plane */ zfar?: number }
  | { type: "orthographic"; /** half the view height; default 1 */ ymag?: number; znear?: number; zfar?: number };

/** How `loadTexture` makes a texture. */
export interface TextureOptions {
  /** "srgb" (default) for colors — baseColor, emissive; "linear" for data
   *  — normal, metallic-roughness, occlusion. */
  colorSpace?: "srgb" | "linear";
  /** Default "repeat". */
  wrap?: "repeat" | "clamp" | "mirror";
  /** Build mipmaps (trilinear filtering); default true. */
  mipmaps?: boolean;
  /** Flip rows on upload; default false (glTF's convention). */
  flipY?: boolean;
}

/** How `setMaterialTexture` uses a texture. */
export interface MaterialTextureOptions {
  /** Which TEXCOORD set the map reads: 0 (default) or 1. */
  texCoord?: number;
  /** KHR_texture_transform: UV offset (default [0, 0]). */
  offset?: [number, number];
  /** UV scale, i.e. repeats (default [1, 1]). */
  scale?: [number, number];
  /** UV rotation in radians (default 0). */
  rotation?: number;
}

/** `setToneMapping`'s names. */
export type ToneMappingName = "none" | "neutral" | "aces" | "agx";

/** `setQuality`'s presets. */
export type QualityName = "low" | "medium" | "high";

/** `setShadingModel`'s names. */
export type ShadingModelName = "openpbr" | "gltf";

/** Handles in a typed array (`cameras`, `lights`, `materials`): reading an
 *  element gives the handle type; it is a `Uint32Array` at run time. */
export type Handles<T extends number> = Uint32Array & { readonly [index: number]: T };

/** What `pick` found. */
export interface PickHit {
  /** The mesh entity (its `parent` is the glTF node). */
  entity: Entity;
  /** World position hit. */
  point: Float32Array;
  /** From the camera. */
  distance: number;
}

/** What a mesh entity draws with: renderer handles, not owned by the scene. */
export interface MeshOptions {
  program: ProgramId;
  state: StateId;
  /** 0 or absent: no vertex array (vertex-pulling shaders). */
  vao?: VertexArrayId;
  /** Group slot 1; 0 or absent: none. */
  material?: ParamGroupId;
  /** Blended: drawn after opaque meshes, farthest first. */
  transparent?: boolean;
  /** Sees the opaque scene behind it (HDR output; scene_transmitted). */
  transmissive?: boolean;
}
"#;

fn get(obj: &JsValue, key: &str) -> Result<Option<JsValue>, Error> {
    // Absent options: every key takes its default.
    if obj.is_undefined() || obj.is_null() {
        return Ok(None);
    }
    if !obj.is_object() {
        return Err("expected an options object".into());
    }
    let v = Reflect::get(obj, &key.into()).map_err(|_| Error::from(format!("can't read {key}")))?;
    Ok((!v.is_undefined() && !v.is_null()).then_some(v))
}

pub(crate) fn f32_or(obj: &JsValue, key: &str, default: f32) -> Result<f32, Error> {
    match get(obj, key)? {
        None => Ok(default),
        // NaN and the infinities would reach the shaders (and TAA's
        // history) as they are.
        Some(v) => v
            .as_f64()
            .filter(|x| x.is_finite())
            .map(|x| x as f32)
            .ok_or_else(|| format!("\"{key}\" must be a finite number").into()),
    }
}

/// Whether `values` are all finite; if not, says so (once per setter) and
/// the setter leaves things as they were — NaN or an infinity would reach
/// the shaders, and TAA's history would keep it.
pub(crate) fn finite(op: &str, values: &[f32]) -> bool {
    if values.iter().all(|x| x.is_finite()) {
        return true;
    }
    luxel_gl::console::error_once(&format!("[luxel] {op}: not a finite number; ignored"));
    false
}

pub(crate) fn u32_or(obj: &JsValue, key: &str, default: u32) -> Result<u32, Error> {
    // As f64: a handle (generation << 20 | index) past 2^24 isn't exact in
    // f32, and would round to another object's.
    let x = match get(obj, key)? {
        None => return Ok(default),
        Some(v) => v
            .as_f64()
            .ok_or_else(|| Error::from(format!("\"{key}\" must be a number")))?,
    };
    if !(0.0..=f64::from(u32::MAX)).contains(&x) || x.fract() != 0.0 {
        return Err(format!("\"{key}\" must be a handle (a non-negative integer)").into());
    }
    Ok(x as u32)
}

pub(crate) fn bool_or(obj: &JsValue, key: &str, default: bool) -> Result<bool, Error> {
    match get(obj, key)? {
        None => Ok(default),
        Some(v) => v
            .as_bool()
            .ok_or_else(|| format!("\"{key}\" must be true or false").into()),
    }
}

pub(crate) fn string(obj: &JsValue, key: &str) -> Result<String, Error> {
    get(obj, key)?
        .and_then(|v| v.as_string())
        .ok_or_else(|| format!("\"{key}\" must be a string").into())
}

pub(crate) fn vec3_or(obj: &JsValue, key: &str, default: Vec3) -> Result<Vec3, Error> {
    match get(obj, key)? {
        None => Ok(default),
        Some(v) => {
            let n =
                numbers(&v).ok_or_else(|| Error::from(format!("\"{key}\" must be 3 numbers")))?;
            match n.as_slice() {
                [x, y, z] => Ok(Vec3::new(*x, *y, *z)),
                _ => Err(format!("\"{key}\" must be 3 numbers").into()),
            }
        }
    }
}

/// A number or an array-like of numbers as floats (None for anything else).
pub(crate) fn numbers(v: &JsValue) -> Option<Vec<f32>> {
    if let Some(x) = v.as_f64() {
        return x.is_finite().then(|| vec![x as f32]);
    }
    if !v.is_object() {
        return None;
    }
    let len = Reflect::get(v, &"length".into()).ok()?.as_f64()? as u32;
    (0..len)
        .map(|i| {
            Reflect::get_u32(v, i)
                .ok()?
                .as_f64()
                .filter(|x| x.is_finite())
                .map(|x| x as f32)
        })
        .collect()
}

pub(crate) fn string_or(obj: &JsValue, key: &str, default: &str) -> Result<String, Error> {
    match get(obj, key)? {
        None => Ok(default.to_string()),
        Some(v) => v
            .as_string()
            .ok_or_else(|| format!("\"{key}\" must be a string").into()),
    }
}

// A pair of numbers at `key`, or `default` when absent.
pub(crate) fn vec2_or(obj: &JsValue, key: &str, default: [f32; 2]) -> Result<[f32; 2], Error> {
    match get(obj, key)? {
        None => Ok(default),
        Some(v) => match numbers(&v).as_deref() {
            Some(&[x, y]) => Ok([x, y]),
            _ => Err(format!("\"{key}\" needs 2 numbers").into()),
        },
    }
}
