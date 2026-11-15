//! Cameras, lights and the environment for JS.

use glam::Vec3;
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
        let aspect = self.world.get::<Camera>(e).map_or(1.0, |c| c.aspect);
        let near = options::f32_or(&camera, "znear", 0.1)?;
        let far = options::f32_or(&camera, "zfar", 1000.0)?;
        if !(near > 0.0 && far > near) {
            return Err("setCamera: needs 0 < znear < zfar".into());
        }
        let cam = match options::string(&camera, "type")?.as_str() {
            "perspective" => {
                let fov = options::f32_or(&camera, "yfov", std::f32::consts::FRAC_PI_4)?;
                Camera::perspective(fov, aspect, near, far)
            }
            "orthographic" => {
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
        match self.world.get_mut::<Camera>(e) {
            Some(mut c) => c.aspect = aspect,
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

    /// Frozen shadows, for a scene that won't move: drawn once — a
    /// directional light's fitted to every caster rather than split along
    /// the view, so they hold wherever the camera goes — then kept, with
    /// no shadow pass, until unfrozen, `invalidateShadows`, or an edit of
    /// lights, materials, models or visibility through the scene. Default
    /// off.
    #[wasm_bindgen(js_name = setShadowsFrozen)]
    pub fn set_shadows_frozen(&mut self, on: bool) {
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
        self.renderer.invalidate_shadows();
    }

    /// Bytes the shadow maps take now (all layers).
    #[wasm_bindgen(js_name = shadowMemory)]
    pub fn shadow_memory(&self) -> f64 {
        self.renderer.shadow_bytes() as f64
    }

    #[wasm_bindgen(js_name = removeLight)]
    pub fn remove_light(&mut self, e: Entity) {
        // Frozen shadows are baked again with it.
        self.renderer.rebake_shadows();
        self.world.remove::<Light>(e);
    }

    /// Scales radiance before tone mapping. Default 1.
    #[wasm_bindgen(js_name = setExposure)]
    pub fn set_exposure(&mut self, exposure: f32) {
        self.renderer.set_exposure(exposure);
    }

    /// Geometric specular antialiasing: where the normal turns within a
    /// pixel, roughness grows (by up to `threshold` in α², `variance`
    /// scaling how fast), so sharp highlights on curved smooth surfaces
    /// don't sparkle. Higher is calmer and softer; variance 0 turns it off.
    /// Default off (0, 0.2); Filament's is (0.15, 0.2).
    #[wasm_bindgen(js_name = setSpecularAntialiasing)]
    pub fn set_specular_antialiasing(&mut self, variance: f32, threshold: f32) {
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
        if self.compact_environment == on {
            return Ok(());
        }
        self.compact_environment = on;
        if let Some((bytes, size)) = self.env_source.clone() {
            self.sync_context();
            let image = decode_hdr_rgbe(&bytes)?;
            self.start_environment(&image, size)?;
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
        self.renderer.set_environment_rotation(radians);
    }

    /// Shows the environment behind the scene, `blur` 0 (sharp) … 1.
    #[wasm_bindgen(js_name = setBackground)]
    pub fn set_background(&mut self, blur: f32) {
        self.renderer.set_background(Some(blur));
        // Start compiling now; `whenReady` waits for it.
        self.renderer.background_program(&self.ctx);
    }

    /// No background: the clear color shows.
    #[wasm_bindgen(js_name = clearBackground)]
    pub fn clear_background(&mut self) {
        self.renderer.set_background(None);
    }
}

impl Scene {
    /// `setCamera` from Rust.
    pub fn insert_camera(&mut self, e: Entity, camera: Camera) {
        self.insert(e, "setCamera", camera);
    }

    /// `setLight` from Rust.
    pub fn insert_light(&mut self, e: Entity, light: Light) {
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
