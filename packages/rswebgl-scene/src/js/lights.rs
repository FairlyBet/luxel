//! Cameras, lights and the environment for JS.

use glam::{Vec3, Vec4};
use rswebgl::console;
use rswebgl::error::Error;
use rswebgl_ecs::Entity;
use wasm_bindgen::prelude::*;

use super::{Scene, options};
use crate::camera::Camera;
use crate::environment::{EnvBaker, PendingEnvironment};
use crate::hdr::{RgbeImage, decode_hdr_rgbe};
use crate::lighting::{Light, LightShadow};

#[wasm_bindgen]
impl Scene {
    // --- cameras --------------------------------------------------------------

    /// Makes `e` a camera (see `CameraOptions`; glTF camera names). It looks
    /// down its local −Z: place it with `lookAt` or the transform setters.
    /// `renderFrame` keeps its aspect in step with the canvas. Throws on bad
    /// options.
    #[wasm_bindgen(js_name = setCamera)]
    pub fn set_camera(
        &mut self,
        e: Entity,
        #[wasm_bindgen(unchecked_param_type = "CameraOptions")] camera: JsValue,
    ) -> Result<(), Error> {
        self.invalidate();
        let aspect = self.world.get::<Camera>(e).map_or(1.0, |c| c.aspect);
        let near = options::f32_or(&camera, "znear", 0.1)?;
        // Infinity: no far plane (a glTF camera without zfar; perspective
        // only).
        let infinite = js_sys::Reflect::get(&camera, &"zfar".into())
            .ok()
            .and_then(|v| v.as_f64())
            == Some(f64::INFINITY);
        let far = if infinite {
            f32::INFINITY
        } else {
            options::f32_or(&camera, "zfar", 1000.0)?
        };
        if !(near > 0.0 && far > near) {
            return Err("setCamera: needs 0 < znear < zfar".into());
        }
        let cam = match options::string(&camera, "type")?.as_str() {
            "perspective" => {
                let fov = options::f32_or(&camera, "yfov", std::f32::consts::FRAC_PI_4)?;
                Camera::perspective(fov, aspect, near, far)
            }
            "orthographic" => {
                if infinite {
                    return Err("setCamera: an orthographic camera needs a finite zfar".into());
                }
                let ymag = options::f32_or(&camera, "ymag", 1.0)?;
                Camera::orthographic(ymag, aspect, near, far)
            }
            other => {
                return Err(format!(
                    "setCamera: type \"{other}\" isn't \"perspective\" or \"orthographic\""
                )
                .into());
            }
        };
        self.insert_camera(e, cam);
        Ok(())
    }

    /// Width / height of the target, for cameras used with `render` into a
    /// pass of yours (`renderFrame` sets it from the canvas).
    #[wasm_bindgen(js_name = setCameraAspect)]
    pub fn set_camera_aspect(&mut self, e: Entity, aspect: f32) {
        if !options::finite("setCameraAspect", &[aspect]) {
            return;
        }
        match self.world.get_mut::<Camera>(e) {
            // Only a different aspect is a change (renderFrame sets it each frame).
            Some(mut c) => c.set_if_neq(Camera { aspect, ..*c }),
            None => console::error_once(
                "[rswebgl-scene] setCameraAspect: the entity is not a camera; ignored",
            ),
        }
    }

    // --- lights ---------------------------------------------------------------

    /// Makes `e` a punctual light (see `LightOptions`; KHR_lights_punctual
    /// names and units). Directional and spot lights shine down the entity's
    /// −Z: aim them with `lookAt`. Up to 8 lights reach the shaders. Throws on
    /// bad options.
    #[wasm_bindgen(js_name = setLight)]
    pub fn set_light(
        &mut self,
        e: Entity,
        #[wasm_bindgen(unchecked_param_type = "LightOptions")] light: JsValue,
    ) -> Result<(), Error> {
        self.invalidate();
        // Frozen shadows are baked again with it.
        self.renderer.rebake_shadows();
        let color = options::vec3_or(&light, "color", Vec3::ONE)?;
        let intensity = options::f32_or(&light, "intensity", 1.0)?;
        let range = options::f32_or(&light, "range", 0.0)?;
        let l = match options::string(&light, "type")?.as_str() {
            "directional" => Light::directional(color, intensity),
            "point" => Light::point(color, intensity, range),
            "spot" => {
                let inner = options::f32_or(&light, "innerConeAngle", 0.0)?;
                let outer = options::f32_or(&light, "outerConeAngle", std::f32::consts::FRAC_PI_4)?;
                if !(0.0 <= inner && inner < outer && outer <= std::f32::consts::FRAC_PI_2) {
                    return Err("setLight: needs 0 ≤ innerConeAngle < outerConeAngle ≤ π/2".into());
                }
                Light::spot(color, intensity, range, inner, outer)
            }
            other => {
                return Err(format!(
                    "setLight: type \"{other}\" isn't \"directional\", \"point\" or \"spot\""
                )
                .into());
            }
        };
        let cast = options::bool_or(&light, "castShadow", false)?;
        if cast {
            let d = LightShadow::default();
            let shadow = LightShadow {
                normal_bias: options::f32_or(&light, "shadowNormalBias", d.normal_bias)?,
                slope_bias: options::f32_or(&light, "shadowSlopeBias", d.slope_bias)?,
                constant_bias: options::f32_or(&light, "shadowConstantBias", d.constant_bias)?,
                cascades: options::u32_or(&light, "shadowCascades", d.cascades)?,
                split_lambda: options::f32_or(&light, "shadowSplitLambda", d.split_lambda)?,
                max_distance: options::f32_or(&light, "shadowMaxDistance", d.max_distance)?,
                map_size: options::u32_or(&light, "shadowMapSize", d.map_size)?,
            };
            if !(1..=crate::lighting::MAX_CASCADES as u32).contains(&shadow.cascades) {
                return Err("setLight: shadowCascades is 1 to 4".into());
            }
            self.world.insert(e, shadow);
        } else {
            self.world.remove::<LightShadow>(e);
        }
        self.insert_light(e, l);
        Ok(())
    }

    /// The side of each shadow map in texels: 2048 by default; rounded to
    /// a power of two in 256..=8192 (and the device's limit). One depth
    /// layer per shadow-casting light, 4 bytes a texel (2048: 16 MB each;
    /// 2 bytes with `setShadowDepthBits(16)`).
    #[wasm_bindgen(js_name = setShadowMapSize)]
    pub fn set_shadow_map_size(&mut self, size: u32) {
        self.invalidate();
        self.renderer.set_shadow_map_size(size);
    }

    #[wasm_bindgen(js_name = shadowMapSize)]
    pub fn shadow_map_size(&self) -> u32 {
        self.renderer.shadow_map_size()
    }

    /// The shadow maps' depth precision: 24 bits (the default) or 16 —
    /// half the memory (a 2048 map: 8 MB instead of 16), coarser depth
    /// steps: enough for a model or a room; over long distances shadow
    /// acne comes sooner (raise the biases). Throws on other values.
    #[wasm_bindgen(js_name = setShadowDepthBits)]
    pub fn set_shadow_depth_bits(&mut self, bits: u32) -> Result<(), Error> {
        self.invalidate();
        match bits {
            16 | 24 => {
                self.renderer.set_shadow_depth16(bits == 16);
                Ok(())
            }
            _ => Err("setShadowDepthBits: 16 or 24".into()),
        }
    }

    #[wasm_bindgen(js_name = shadowDepthBits)]
    pub fn shadow_depth_bits(&self) -> u32 {
        if self.renderer.shadow_depth16() {
            16
        } else {
            24
        }
    }

    /// Tints what each shadow cascade shades — red, green, blue, yellow,
    /// nearest first — to see where they split. Default off.
    #[wasm_bindgen(js_name = setShadowCascadeDebug)]
    pub fn set_shadow_cascade_debug(&mut self, on: bool) {
        self.invalidate();
        self.renderer.set_debug_cascades(on);
    }

    /// Shadow maps in use last frame: one per spot light, one per cascade
    /// of a directional light.
    #[wasm_bindgen(js_name = shadowMaps)]
    pub fn shadow_maps(&self) -> u32 {
        self.renderer.shadow_views() as u32
    }

    /// Of those, the ones drawn last frame: a map whose light and casters
    /// didn't move (nor deform, nor change mesh or material) is kept.
    #[wasm_bindgen(js_name = shadowMapsDrawn)]
    pub fn shadow_maps_drawn(&self) -> u32 {
        self.renderer.shadow_views_drawn() as u32
    }

    /// Draw calls into the shadow maps last frame: identical meshes (of a
    /// shader that reads its instance) drawn as one per map.
    #[wasm_bindgen(js_name = shadowDrawCalls)]
    pub fn shadow_draw_calls(&self) -> u32 {
        self.renderer.shadow_draw_calls() as u32
    }

    /// A soft shadow on an invisible ground under the shadow casters, for
    /// scenes lit by their environment alone (three.js' contact shadows):
    /// `GroundShadowOptions`, or null to turn it off (off by default). The
    /// casters are seen from below and their nearness to the ground blurred
    /// into a dark patch, redrawn only when they move.
    #[wasm_bindgen(js_name = setGroundShadow)]
    pub fn set_ground_shadow(
        &mut self,
        #[wasm_bindgen(unchecked_param_type = "GroundShadowOptions | null")] options: JsValue,
    ) -> Result<(), Error> {
        self.invalidate();
        self.renderer.rebake_shadows();
        if options.is_null() || options.is_undefined() {
            self.renderer.set_ground_shadow(None);
            return Ok(());
        }
        let d = crate::lighting::GroundShadow::default();
        let s = crate::lighting::GroundShadow {
            darkness: options::f32_or(&options, "darkness", d.darkness)?,
            blur: options::f32_or(&options, "blur", d.blur)?,
            opacity: options::f32_or(&options, "opacity", d.opacity)?,
            reach: options::f32_or(&options, "reach", d.reach)?,
            size: options::u32_or(&options, "size", d.size)?,
        };
        if !(s.darkness >= 0.0
            && s.blur >= 0.0
            && (0.0..=1.0).contains(&s.opacity)
            && s.reach >= 0.0)
        {
            return Err("setGroundShadow: darkness, blur, reach ≥ 0, opacity in 0..1".into());
        }
        self.renderer.set_ground_shadow(Some(s));
        Ok(())
    }

    /// The ground shadow's options as `setGroundShadow` takes them, or null
    /// when it is off.
    #[wasm_bindgen(js_name = groundShadow, unchecked_return_type = "GroundShadowOptions | null")]
    pub fn ground_shadow(&self) -> JsValue {
        self.renderer.ground_shadow().map_or(JsValue::NULL, |g| {
            super::getters::object(&[
                ("darkness", g.darkness.into()),
                ("blur", g.blur.into()),
                ("opacity", g.opacity.into()),
                ("reach", g.reach.into()),
                ("size", g.size.into()),
            ])
        })
    }

    /// Whether the last frame drew the ground shadow again (its casters
    /// moved; kept otherwise).
    #[wasm_bindgen(js_name = groundShadowDrawn)]
    pub fn ground_shadow_drawn(&self) -> bool {
        self.renderer.ground_shadow_drawn()
    }

    /// Frozen shadows, for a scene that won't move: drawn once — a
    /// directional light's fitted to every caster rather than split along
    /// the view, so they hold wherever the camera goes — then kept, with
    /// no shadow pass, until unfrozen, `invalidateShadows`, or an edit of
    /// lights, materials, models or visibility through the scene. Default
    /// off.
    #[wasm_bindgen(js_name = setShadowsFrozen)]
    pub fn set_shadows_frozen(&mut self, on: bool) {
        self.invalidate();
        self.renderer.set_shadows_frozen(on);
    }

    #[wasm_bindgen(js_name = shadowsFrozen)]
    pub fn shadows_frozen(&self) -> bool {
        self.renderer.shadows_frozen()
    }

    /// Draws the shadow maps again next frame (frozen ones too): after
    /// changes the scene doesn't see, such as raw GL writes into a texture
    /// an alpha-masked material reads.
    #[wasm_bindgen(js_name = invalidateShadows)]
    pub fn invalidate_shadows(&mut self) {
        self.invalidate();
        self.renderer.invalidate_shadows();
    }

    /// Bytes the shadow maps take now (all layers).
    #[wasm_bindgen(js_name = shadowMemory)]
    pub fn shadow_memory(&self) -> f64 {
        self.renderer.shadow_bytes() as f64
    }

    #[wasm_bindgen(js_name = removeLight)]
    pub fn remove_light(&mut self, e: Entity) {
        self.invalidate();
        // Frozen shadows are baked again with it.
        self.renderer.rebake_shadows();
        self.world.remove::<Light>(e);
    }

    /// Scales radiance before tone mapping. Default 1.
    #[wasm_bindgen(js_name = setExposure)]
    pub fn set_exposure(&mut self, exposure: f32) {
        self.invalidate();
        if !options::finite("setExposure", &[exposure]) {
            return;
        }
        self.renderer.set_exposure(exposure);
    }

    /// Geometric specular antialiasing: where the normal turns within a
    /// pixel, roughness grows (by up to `threshold` in α², `variance`
    /// scaling how fast), so sharp highlights on curved smooth surfaces
    /// don't sparkle. Higher is calmer and softer; variance 0 turns it off.
    /// Default off (0, 0.2); Filament's is (0.15, 0.2).
    #[wasm_bindgen(js_name = setSpecularAntialiasing)]
    pub fn set_specular_antialiasing(&mut self, variance: f32, threshold: f32) {
        self.invalidate();
        if !options::finite("setSpecularAntialiasing", &[variance, threshold]) {
            return;
        }
        self.renderer.set_specular_antialiasing(variance, threshold);
    }

    /// `[variance, threshold]` of the specular antialiasing.
    #[wasm_bindgen(js_name = specularAntialiasing)]
    pub fn specular_antialiasing(&self) -> Vec<f32> {
        let (v, t) = self.renderer.specular_antialiasing();
        vec![v, t]
    }

    // --- environment ----------------------------------------------------------

    /// Scales the environment's light (linear RGB). Without an environment
    /// (the default white one) it's a flat ambient light: default 0.2.
    /// `setEnvironmentHdr` resets it to 1 (an intensity set while that bakes
    /// is kept).
    #[wasm_bindgen(js_name = setEnvironmentIntensity)]
    pub fn set_environment_intensity(&mut self, r: f32, g: f32, b: f32) {
        self.invalidate();
        if !options::finite("setEnvironmentIntensity", &[r, g, b]) {
            return;
        }
        let rgb = Vec3::new(r, g, b);
        self.renderer.set_environment_intensity(rgb);
        if self.env_pending.is_some() {
            self.env_intensity_after_bake = Some(rgb);
        }
    }

    /// Lights the scene with an HDR environment: the bytes of a Radiance
    /// `.hdr` equirectangular image (fetch it yourself), baked on the GPU into
    /// cube maps `size` texels per face (256 is a good default). The file is
    /// read and uploaded now (throws if it can't be read); the bake runs in
    /// the background over the next few frames — the previous environment
    /// shows until it's done, and `whenReady` waits for it. A bake that fails
    /// (no half-float render targets) is reported on the console.
    /// The bytes are kept (a copy) to bake it again after a context loss.
    #[wasm_bindgen(js_name = setEnvironmentHdr)]
    pub fn set_environment_hdr(&mut self, bytes: &[u8], size: u32) -> Result<(), Error> {
        self.invalidate();
        self.sync_context();
        let image = decode_hdr_rgbe(bytes)?;
        self.start_environment(&image, size)?;
        // Kept to bake again after a context loss.
        self.env_source = Some((bytes.to_vec(), size));
        Ok(())
    }

    /// Whether environments bake into R11F_G11F_B10F cube maps (on by
    /// default, as Filament's IBL prefilter): half the memory of RGBA16F
    /// (a 256 environment's specular cube: ~2.1 MB instead of ~4.2), ~1.5%
    /// color steps — on devices that render to it (`EXT_color_buffer_float`;
    /// RGBA16F otherwise); then, where the device has BPTC (desktops), the
    /// cubes are compressed to BC6H on the GPU once baked: a byte a texel
    /// (~0.5 MB for that cube). Off: RGBA16F (full half-float precision). A
    /// change bakes the current environment again.
    #[wasm_bindgen(js_name = setCompactEnvironment)]
    pub fn set_compact_environment(&mut self, on: bool) -> Result<(), Error> {
        self.invalidate();
        if self.compact_environment == on {
            return Ok(());
        }
        self.compact_environment = on;
        if let Some((bytes, size)) = self.env_source.clone() {
            self.sync_context();
            // The bake comes back at the intensity it has now (or was to
            // get once baked), not the default a new environment starts at.
            let keep = self
                .env_intensity_after_bake
                .unwrap_or_else(|| self.renderer.environment_intensity());
            let image = decode_hdr_rgbe(&bytes)?;
            self.start_environment(&image, size)?;
            self.env_intensity_after_bake = Some(keep);
        }
        Ok(())
    }

    /// See `setCompactEnvironment`.
    #[wasm_bindgen(js_name = compactEnvironment)]
    pub fn compact_environment(&self) -> bool {
        self.compact_environment
    }

    /// Back to the default: a flat white ambient light at intensity 0.2.
    #[wasm_bindgen(js_name = clearEnvironment)]
    pub fn clear_environment(&mut self) {
        self.invalidate();
        self.sync_context();
        self.env_source = None;
        if let Some(p) = self.env_pending.take() {
            p.cancel();
        }
        self.renderer.clear_environment(&self.ctx);
    }

    /// Whether an HDR environment is set (or baking).
    #[wasm_bindgen(js_name = hasEnvironment)]
    pub fn has_environment(&self) -> bool {
        self.renderer.has_environment() || self.env_pending.is_some()
    }

    /// Turns the environment (light and background) about +Y, in radians.
    #[wasm_bindgen(js_name = setEnvironmentRotation)]
    pub fn set_environment_rotation(&mut self, radians: f32) {
        self.invalidate();
        if !options::finite("setEnvironmentRotation", &[radians]) {
            return;
        }
        self.renderer.set_environment_rotation(radians);
    }

    /// Shows the environment behind the scene, `blur` 0 (sharp) … 1 (in
    /// place of a background color or gradient).
    #[wasm_bindgen(js_name = setBackground)]
    pub fn set_background(&mut self, blur: f32) {
        self.invalidate();
        if !options::finite("setBackground", &[blur]) {
            return;
        }
        self.renderer.set_fill([Vec4::ZERO; 2]);
        self.renderer.set_background(Some(blur));
        // Start compiling now; `whenReady` waits for it.
        self.renderer.background_program(&self.ctx);
    }

    /// No background — no environment, color or gradient: the canvas's
    /// clear color shows.
    #[wasm_bindgen(js_name = clearBackground)]
    pub fn clear_background(&mut self) {
        self.invalidate();
        self.renderer.set_background(None);
        self.renderer.set_fill([Vec4::ZERO; 2]);
    }

    /// A flat color behind the scene, in display values (sRGB 0–1, as CSS
    /// colors): shown exactly, neither exposed nor tone mapped — white stays
    /// white. Glass sees it. Replaces the environment background.
    #[wasm_bindgen(js_name = setBackgroundColor)]
    pub fn set_background_color(
        &mut self,
        #[wasm_bindgen(unchecked_param_type = "ArrayLike<number>")] color: &[f32],
    ) -> Result<(), Error> {
        let c = fill_color("setBackgroundColor", color)?;
        self.set_fill(c.extend(1.0), Vec4::ZERO);
        Ok(())
    }

    /// A gradient behind the scene, as `setBackgroundColor` (display values,
    /// shown exactly): "vertical" from `from` at the bottom to `to` at the
    /// top, "radial" from `from` at the center to `to` at the corners;
    /// mixed in sRGB, as CSS gradients are.
    #[wasm_bindgen(js_name = setBackgroundGradient)]
    pub fn set_background_gradient(
        &mut self,
        #[wasm_bindgen(unchecked_param_type = "ArrayLike<number>")] from: &[f32],
        #[wasm_bindgen(unchecked_param_type = "ArrayLike<number>")] to: &[f32],
        #[wasm_bindgen(unchecked_param_type = "\"vertical\" | \"radial\"")] kind: &str,
    ) -> Result<(), Error> {
        let a = fill_color("setBackgroundGradient", from)?;
        let b = fill_color("setBackgroundGradient", to)?;
        let w = match kind {
            "vertical" => 2.0,
            "radial" => 3.0,
            other => {
                return Err(format!(
                    "setBackgroundGradient: kind \"{other}\" isn't \"vertical\" or \"radial\""
                )
                .into());
            }
        };
        self.set_fill(a.extend(w), b.extend(0.0));
        Ok(())
    }

    /// The background color or gradient (`setBackgroundColor`,
    /// `setBackgroundGradient`), or undefined without one.
    #[wasm_bindgen(js_name = backgroundFill, unchecked_return_type = "BackgroundFill | undefined")]
    pub fn background_fill(&self) -> JsValue {
        let [a, b] = self.renderer.fill();
        let kind = match a.w as u32 {
            1 => "color",
            2 => "vertical",
            3 => "radial",
            _ => return JsValue::UNDEFINED,
        };
        let o = js_sys::Object::new();
        let set = |k: &str, v: JsValue| {
            let _ = js_sys::Reflect::set(&o, &k.into(), &v);
        };
        set("type", kind.into());
        set("from", js_sys::Float32Array::from(&a.truncate().to_array()[..]).into());
        if kind != "color" {
            set("to", js_sys::Float32Array::from(&b.truncate().to_array()[..]).into());
        }
        o.into()
    }
}

// A background color: three (or four, alpha ignored) finite numbers,
// clamped to 0–1.
fn fill_color(what: &str, c: &[f32]) -> Result<Vec3, Error> {
    if !(3..=4).contains(&c.len()) || c.iter().any(|v| !v.is_finite()) {
        return Err(format!("{what}: a color is [r, g, b], finite, 0–1").into());
    }
    Ok(Vec3::new(c[0], c[1], c[2]).clamp(Vec3::ZERO, Vec3::ONE))
}

impl Scene {
    // A background color or gradient, in place of the environment
    // background.
    fn set_fill(&mut self, a: Vec4, b: Vec4) {
        self.invalidate();
        self.renderer.set_background(None);
        self.renderer.set_fill([a, b]);
        // Straight to the canvas it draws with the background shader.
        self.renderer.background_program(&self.ctx);
    }

    /// `setCamera` from Rust.
    pub fn insert_camera(&mut self, e: Entity, camera: Camera) {
        self.invalidate();
        self.insert(e, "setCamera", camera);
    }

    /// `setLight` from Rust.
    pub fn insert_light(&mut self, e: Entity, light: Light) {
        self.invalidate();
        self.insert(e, "setLight", light);
    }
}

impl Scene {
    // Starts baking `image` in the background (the bake shaders made on
    // first use), cancelling a bake still running.
    pub(crate) fn start_environment(&mut self, image: &RgbeImage, size: u32) -> Result<(), Error> {
        if let Some(p) = self.env_pending.take() {
            p.cancel();
        }
        self.env_intensity_after_bake = None;
        let baker = match &self.env_baker {
            Some(b) => b.clone(),
            None => {
                let b = std::rc::Rc::new(EnvBaker::new(&self.ctx)?);
                self.env_baker = Some(b.clone());
                b
            }
        };
        self.env_pending = Some(PendingEnvironment::start(
            &self.ctx,
            baker,
            image,
            size,
            self.compact_environment,
        )?);
        Ok(())
    }

    // Swaps in a bake that has finished (at most once a frame).
    pub(crate) fn apply_environment(&mut self) {
        let Some(result) = self.env_pending.as_ref().and_then(|p| p.take()) else {
            return;
        };
        self.env_pending = None;
        self.ctx.note_content_changed();
        match result {
            Ok(env) => {
                if self.compact_environment {
                    self.compress_environment(env.cubes());
                }
                self.renderer.set_environment(&self.ctx, env);
                if let Some(rgb) = self.env_intensity_after_bake.take() {
                    self.renderer.set_environment_intensity(rgb);
                }
            }
            Err(e) => console::error(&format!("[rswebgl-scene] environment: {e}")),
        }
    }

    // BC6H for the environment's cubes where the device has BPTC: once the
    // encoder has compiled, in place (the renderer keeps its handles).
    fn compress_environment(&self, cubes: [rswebgl::ids::TextureId; 2]) {
        use rswebgl::texture_compressor::{CompressKind, TextureCompressor};
        let comp = match TextureCompressor::shared(&self.ctx) {
            Ok(c) if c.supports(CompressKind::Hdr) => c,
            Ok(_) => return,
            Err(e) => {
                console::warn(&format!(
                    "[rswebgl-scene] environment left uncompressed: {e}"
                ));
                return;
            }
        };
        let ctx = self.ctx.clone();
        wasm_bindgen_futures::spawn_local(async move {
            let _ = wasm_bindgen_futures::JsFuture::from(comp.when_ready()).await;
            for t in cubes {
                if ctx.texture_is_valid(t) {
                    comp.compress(t, CompressKind::Hdr);
                }
            }
        });
    }
}
