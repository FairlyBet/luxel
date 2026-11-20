//! How `renderFrame` gets to the canvas: through the HDR target and display
//! pass (`crate::output`) where the device can render half floats, else
//! straight, with each shader tone mapping its own pixels.

use luxel_gl::console;
use luxel_gl::error::Error;
use luxel_gl::pass::Pass;
use wasm_bindgen::prelude::*;

use super::{Scene, options};
use crate::output::{HdrOutput, ToneMapping};

#[wasm_bindgen]
impl Scene {
    /// A preset of the settings that trade quality for speed — a starting
    /// point, each still settable after:
    /// - "low": no MSAA or TAA, FXAA; 1024² shadow maps; no ambient
    ///   occlusion or contact shadows; transmission at half resolution; at
    ///   most 1 pixel per CSS pixel.
    /// - "medium": 4× MSAA; 2048² shadow maps; no ambient occlusion or
    ///   contact shadows; transmission at half resolution; at most 1.5.
    /// - "high": TAA; 2048² shadow maps; ambient occlusion; full-resolution
    ///   transmission; at most 2.
    ///
    /// Contact shadows stay off in every preset (`setContactShadows` turns
    /// them on): screen-space, they show only part of a long shadow and
    /// change with the view; shadow maps and AO look steadier.
    ///
    /// The pixel cap is the canvas's (`defaultFramebuffer().setMaxPixelRatio`,
    /// with auto-resize). Throws on another name.
    #[wasm_bindgen(js_name = setQuality)]
    pub fn set_quality(
        &mut self,
        #[wasm_bindgen(unchecked_param_type = "QualityName")] name: &str,
    ) -> Result<(), Error> {
        let (taa, fxaa, samples, shadow, effects, transmission, ratio) = match name {
            "low" => (false, true, 0, 1024, false, 0.5, 1.0),
            "medium" => (false, false, 4, 2048, false, 0.5, 1.5),
            "high" => (true, false, 4, 2048, true, 1.0, 2.0),
            other => {
                return Err(format!("setQuality: \"{other}\" isn't low, medium or high").into());
            }
        };
        self.set_taa(taa);
        self.set_fxaa(fxaa);
        self.set_samples(samples);
        self.set_shadow_map_size(shadow);
        let on = |on: bool| {
            if on {
                js_sys::Object::new().into()
            } else {
                JsValue::NULL
            }
        };
        self.set_ambient_occlusion(on(effects))?;
        self.set_contact_shadows(JsValue::NULL)?;
        self.set_transmission_resolution(transmission);
        self.ctx.default_framebuffer().set_max_pixel_ratio(ratio);
        Ok(())
    }

    /// Screen-space ambient occlusion (GTAO) over the opaque scene, as a
    /// post-pass (three.js' GTAOPass): `AmbientOcclusionOptions`, or null
    /// to turn it off (off by default). With HDR output only (it reads the
    /// scene's depth). Darkens creases and contacts under environment
    /// light — and, being applied to the lit image, under the lights too.
    #[wasm_bindgen(js_name = setAmbientOcclusion)]
    pub fn set_ambient_occlusion(
        &mut self,
        #[wasm_bindgen(unchecked_param_type = "AmbientOcclusionOptions | null")] options: JsValue,
    ) -> Result<(), Error> {
        (|| -> Result<(), Error> {
            self.invalidate();
            if options.is_null() || options.is_undefined() {
                self.ambient_occlusion = None;
                return Ok(());
            }
            let d = crate::occlusion::AmbientOcclusion::default();
            let s = crate::occlusion::AmbientOcclusion {
                radius: options::f32_or(&options, "radius", d.radius)?,
                thickness: options::f32_or(&options, "thickness", d.thickness)?,
                intensity: options::f32_or(&options, "intensity", d.intensity)?,
                power: options::f32_or(&options, "power", d.power)?,
                half_resolution: options::bool_or(&options, "halfResolution", d.half_resolution)?,
            };
            if !(s.radius >= 0.0
                && s.thickness >= 0.0
                && (0.0..=1.0).contains(&s.intensity)
                && s.power > 0.0)
            {
                return Err(
                    "setAmbientOcclusion: radius, thickness ≥ 0, intensity in 0..1, power > 0"
                        .into(),
                );
            }
            self.ambient_occlusion = Some(s);
            Ok(())
        })()
        .map_err(|e| e.in_op("setAmbientOcclusion"))
    }

    /// Screen-space contact shadows of the key light (the one giving the
    /// most light at the scene's centre: directional, point or spot), as
    /// a post-pass over the opaque scene's depth (Filament's): the small
    /// shadows its map is too coarse for, where things touch. `ContactShadowsOptions`,
    /// or null to turn them off (off by default). HDR output only. They take
    /// away that light's share of each pixel only (the opaque pass writes
    /// it beside the color), not the environment's or other lights' — while
    /// every opaque mesh is the built-in PBR's; with a shader of your own
    /// among them (it has no output for the share) the whole pixel is
    /// dimmed, as a plain post-pass would.
    #[wasm_bindgen(js_name = setContactShadows)]
    pub fn set_contact_shadows(
        &mut self,
        #[wasm_bindgen(unchecked_param_type = "ContactShadowsOptions | null")] options: JsValue,
    ) -> Result<(), Error> {
        (|| -> Result<(), Error> {
            self.invalidate();
            if options.is_null() || options.is_undefined() {
                self.contact_shadows = None;
                return Ok(());
            }
            let d = crate::occlusion::ContactShadows::default();
            let s = crate::occlusion::ContactShadows {
                length: options::f32_or(&options, "length", d.length)?,
                thickness: options::f32_or(&options, "thickness", d.thickness)?,
                intensity: options::f32_or(&options, "intensity", d.intensity)?,
                steps: options::u32_or(&options, "steps", d.steps)?,
            };
            if !(s.length >= 0.0
                && s.thickness >= 0.0
                && (0.0..=1.0).contains(&s.intensity)
                && (1..=64).contains(&s.steps))
            {
                return Err(
                    "setContactShadows: length, thickness ≥ 0, intensity in 0..1, steps 1..64"
                        .into(),
                );
            }
            self.contact_shadows = Some(s);
            Ok(())
        })()
        .map_err(|e| e.in_op("setContactShadows"))
    }

    /// The contact shadows' options as `setContactShadows` takes them, or
    /// null when they are off.
    #[wasm_bindgen(js_name = contactShadows, unchecked_return_type = "ContactShadowsOptions | null")]
    pub fn contact_shadows(&self) -> JsValue {
        self.contact_shadows.map_or(JsValue::NULL, |c| {
            super::getters::object(&[
                ("length", c.length.into()),
                ("thickness", c.thickness.into()),
                ("intensity", c.intensity.into()),
                ("steps", c.steps.into()),
            ])
        })
    }

    /// The ambient occlusion's options as `setAmbientOcclusion` takes them,
    /// or null when it is off.
    #[wasm_bindgen(js_name = ambientOcclusion, unchecked_return_type = "AmbientOcclusionOptions | null")]
    pub fn ambient_occlusion(&self) -> JsValue {
        self.ambient_occlusion.map_or(JsValue::NULL, |a| {
            super::getters::object(&[
                ("radius", a.radius.into()),
                ("thickness", a.thickness.into()),
                ("intensity", a.intensity.into()),
                ("power", a.power.into()),
                ("halfResolution", a.half_resolution.into()),
            ])
        })
    }

    /// HDR rendering on or off (on by default where the device renders
    /// half-float targets). On, the scene shades linear radiance into an
    /// offscreen target and tone maps it once; off, it draws straight to the
    /// canvas, each shader tone mapping its pixels (cheaper on bandwidth,
    /// but blending happens after tone mapping). Returns whether HDR is on.
    #[wasm_bindgen(js_name = setHdr)]
    pub fn set_hdr(&mut self, on: bool) -> bool {
        self.invalidate();
        self.sync_context();
        self.last_frame = None;
        self.hdr_wanted = on && HdrOutput::supported(&self.ctx);
        if !self.hdr_wanted {
            if let Some(out) = self.hdr.take() {
                out.destroy(&self.ctx);
            }
            // Its effects need it: their passes go too.
            if let Some(o) = self.occlusion.take() {
                o.destroy(&self.ctx);
            }
        }
        self.hdr_wanted
    }

    /// Whether HDR output is on (`setHdr`).
    pub fn hdr(&self) -> bool {
        self.hdr_wanted
    }

    /// Tone mapping: "neutral" (Khronos PBR Neutral, the default: true
    /// colors, soft highlights), "aces", "agx", or "none" (clip). Throws on
    /// another name.
    #[wasm_bindgen(js_name = setToneMapping)]
    pub fn set_tone_mapping(
        &mut self,
        #[wasm_bindgen(unchecked_param_type = "ToneMappingName")] name: &str,
    ) -> Result<(), Error> {
        self.invalidate();
        self.tone_mapping = ToneMapping::from_name(name).ok_or_else(|| {
            Error::from(format!(
                "setToneMapping: \"{name}\" isn't none, neutral, aces or agx"
            ))
        })?;
        Ok(())
    }

    /// The tone mapping set (`setToneMapping`).
    #[wasm_bindgen(js_name = toneMapping, unchecked_return_type = "ToneMappingName")]
    pub fn tone_mapping(&self) -> String {
        self.tone_mapping.name().to_string()
    }

    /// Temporal antialiasing instead of multisampling (with HDR output):
    /// the camera jitters by a subpixel each frame (Halton 2, 3) and frames
    /// accumulate into a history reprojected through the camera's motion —
    /// and, for meshes that moved (animation, skinning, morph targets, your
    /// own transforms), their own motion vectors — clipped to each pixel's
    /// neighborhood (ported from Bevy and Godot). Smooths edges, thin
    /// geometry and shading aliasing MSAA leaves; pair it with
    /// `setSharpening`. Opaque meshes only: transmissive and transparent
    /// ones draw after it, unjittered, and see the antialiased scene behind
    /// them. Returns whether it is on (needs HDR output).
    #[wasm_bindgen(js_name = setTaa)]
    pub fn set_taa(&mut self, on: bool) -> bool {
        self.invalidate();
        let live = self.sync_context();
        if self.taa != on {
            self.taa = on;
            self.taa_previous = None;
            if let Some(out) = self.hdr.take() {
                out.destroy(&self.ctx); // remade at the next frame
            }
        }
        // Its shader compiling from now (whenReady waits for it); while
        // lost, the restore makes it.
        let on = self.taa();
        if live {
            self.renderer.set_motion_vectors(&self.ctx, on);
        }
        on
    }

    /// Whether TAA is on (`setTaa`; needs HDR output).
    pub fn taa(&self) -> bool {
        self.taa && self.hdr_wanted
    }

    /// FXAA (fast approximate antialiasing, ported from three.js) on the
    /// displayed image, with HDR output: one cheap fullscreen pass that
    /// smooths edges by their contrast — the phone-friendly choice where
    /// MSAA or TAA cost too much (set `setSamples(0)` with it). Softens fine
    /// texture detail a little. Combines with MSAA; with TAA it covers only
    /// what TAA leaves out (transmissive and transparent meshes). Returns
    /// whether it is on (needs HDR output).
    #[wasm_bindgen(js_name = setFxaa)]
    pub fn set_fxaa(&mut self, on: bool) -> bool {
        self.invalidate();
        self.sync_context();
        self.fxaa = on;
        // Compiled now when the target exists, else at the next frame.
        if let Some(out) = &mut self.hdr
            && let Err(e) = out.set_fxaa(&self.ctx, on)
        {
            self.give_up_fxaa(&e);
        }
        self.fxaa()
    }

    /// Whether FXAA is on (`setFxaa`; needs HDR output).
    pub fn fxaa(&self) -> bool {
        self.fxaa && self.hdr_wanted
    }

    /// Auto exposure (with HDR output; ported from Godot 3): each frame's
    /// brightness (its brightest channel, averaged) is measured on the GPU
    /// and the exposure eases toward showing it as `grey` — for scenes lit
    /// in real units (hundreds of lux, a lamp's candela) or that change
    /// from dark to bright. `setExposure` stays on top of it as
    /// compensation. Returns whether it is on.
    #[wasm_bindgen(js_name = setAutoExposure)]
    pub fn set_auto_exposure(&mut self, on: bool) -> bool {
        self.invalidate();
        self.sync_context();
        self.auto_exposure = on;
        if let Some(out) = &mut self.hdr
            && let Err(e) = out.set_auto_exposure(&self.ctx, on)
        {
            self.give_up_auto_exposure(&e);
        }
        self.auto_exposure()
    }

    /// Whether auto exposure is on (`setAutoExposure`; needs HDR output).
    #[wasm_bindgen(js_name = autoExposure)]
    pub fn auto_exposure(&self) -> bool {
        self.auto_exposure && self.hdr_wanted
    }

    /// Auto exposure's settings: `grey` the brightness the frame's average
    /// is shown as (default 0.4), the average's range `min`…`max` it adapts
    /// within (0.05…10000), and `speed`, the share of the way it moves per
    /// second (0.5). Godot's defaults but the ceiling (its 8 can't reach
    /// physically lit scenes).
    #[wasm_bindgen(js_name = setAutoExposureOptions)]
    pub fn set_auto_exposure_options(&mut self, grey: f32, min: f32, max: f32, speed: f32) {
        self.invalidate();
        if !options::finite("setAutoExposureOptions", &[grey, min, max, speed]) {
            return;
        }
        self.auto_exposure_options = [grey, min, max, speed];
    }

    /// Antialiasing samples per pixel of the HDR target (default 4; 0 or 1
    /// for none; lowered to what the device allows). Set the canvas's own
    /// `antialias: false` with HDR on: it isn't used.
    #[wasm_bindgen(js_name = setSamples)]
    pub fn set_samples(&mut self, samples: i32) {
        self.invalidate();
        self.sync_context();
        self.last_frame = None;
        self.samples = samples.max(0);
        if let Some(out) = self.hdr.take() {
            out.destroy(&self.ctx); // remade at the next frame
        }
    }

    /// Dither the displayed colors (default on): hides banding in dark,
    /// smooth gradients at the cost of faint noise.
    #[wasm_bindgen(js_name = setDither)]
    pub fn set_dither(&mut self, on: bool) {
        self.invalidate();
        self.dither = on;
    }

    /// Sharpening of the displayed image, 0 (off, the default) to 1: AMD's
    /// RCAS (FSR 1), which lifts detail without ringing or clipping. Gives
    /// back what TAA's history filtering softens (~0.3–0.6 with `setTaa`).
    /// With HDR output only.
    #[wasm_bindgen(js_name = setSharpening)]
    pub fn set_sharpening(&mut self, amount: f32) {
        self.invalidate();
        if !options::finite("setSharpening", &[amount]) {
            return;
        }
        self.sharpness = amount.clamp(0.0, 1.0);
    }

    pub fn sharpening(&self) -> f32 {
        self.sharpness
    }

    /// The resolution of the opaque-scene copy transmissive surfaces (glass)
    /// see through, as a share of the canvas: 1 (the default) full size,
    /// down to 0.25. Lower saves memory and the copy's time on phones;
    /// what shows through glass gets blurrier. With HDR output.
    #[wasm_bindgen(js_name = setTransmissionResolution)]
    pub fn set_transmission_resolution(&mut self, scale: f32) {
        self.invalidate();
        if !options::finite("setTransmissionResolution", &[scale]) {
            return;
        }
        self.transmission_scale = scale.clamp(0.25, 1.0);
    }

    #[wasm_bindgen(js_name = transmissionResolution)]
    pub fn transmission_resolution(&self) -> f32 {
        self.transmission_scale
    }
}

impl Scene {
    // Points `pass` at the HDR target (made or remade as needed) and the
    // renderer at linear output; or, without HDR, at the canvas with display
    // output. Returns whether HDR is in use this frame.
    pub(super) fn prepare_output(
        &mut self,
        pass: &mut Pass,
        late: &mut Pass,
        w: i32,
        h: i32,
    ) -> bool {
        if self.hdr.as_ref().is_some_and(|o| o.is_lost(&self.ctx))
            && let Some(out) = self.hdr.take()
        {
            out.destroy(&self.ctx);
        }
        if self.hdr_wanted && self.hdr.is_none() {
            let mut made = HdrOutput::new(&self.ctx, self.samples, self.taa);
            // A TAA failure (its shader, a target) costs TAA, not HDR.
            if let Err(e) = &made
                && self.taa
            {
                self.give_up_taa(e);
                made = HdrOutput::new(&self.ctx, self.samples, false);
            }
            match made {
                Ok(out) => self.hdr = Some(out),
                Err(e) => self.give_up_hdr(&e),
            }
        }
        if let Some(out) = &mut self.hdr
            && let Err(e) = out.set_fxaa(&self.ctx, self.fxaa)
        {
            self.give_up_fxaa(&e);
        }
        if let Some(out) = &mut self.hdr {
            match out.set_auto_exposure(&self.ctx, self.auto_exposure) {
                Ok(()) => {
                    let [grey, min, max, speed] = self.auto_exposure_options;
                    out.set_auto_exposure_options(grey, min, max, speed);
                }
                Err(e) => self.give_up_auto_exposure(&e),
            }
        }
        if let Some(out) = &mut self.hdr {
            // Contact shadows dim the key light's share: the opaque pass
            // writes it then (every opaque mesh the built-in PBR's).
            let key_output = self.contact_shadows.is_some()
                && self
                    .world
                    .storage::<crate::render::MeshRenderer>()
                    .is_none_or(|m| {
                        m.data()
                            .iter()
                            .all(|m| m.transparent || m.transmissive || self.pbr.is_pbr(m.program))
                    });
            out.set_key_output(&self.ctx, key_output);
            out.set_transmission_scale(self.transmission_scale);
            out.set_fill(self.renderer.fill());
            match out.prepare_split(&self.ctx, w, h, pass, false) {
                Ok(()) => {
                    out.prepare_late(late);
                    self.renderer.set_output(false, self.tone_mapping);
                    // Through HDR: the direct target isn't needed.
                    if let Some(d) = self.direct.take() {
                        d.destroy(&self.ctx);
                    }
                    return true;
                }
                Err(e) if self.taa => {
                    // Without TAA's targets at the next frame.
                    self.give_up_taa(&e);
                    if let Some(out) = self.hdr.take() {
                        out.destroy(&self.ctx);
                    }
                }
                Err(e) => self.give_up_hdr(&e),
            }
        }
        self.renderer.set_output(true, self.tone_mapping);
        pass.set_target(luxel_gl::ids::FramebufferId::NULL);
        pass.set_clear(&luxel_gl::framebuffer::ClearMask::color_depth());
        pass.clear_resolve();
        pass.set_invalidate(&luxel_gl::framebuffer::InvalidateMask::none());
        false
    }

    // The direct target, made (or remade after a loss) on need, set up for
    // `pass`. False where it can't be: `pass` stays on the canvas then
    // (drawn without a depth test), with a warning once.
    pub(super) fn prepare_direct(&mut self, pass: &mut Pass, w: i32, h: i32) -> bool {
        if self.direct.as_ref().is_some_and(|d| d.is_lost(&self.ctx))
            && let Some(d) = self.direct.take()
        {
            d.destroy(&self.ctx);
        }
        if self.direct.is_none() {
            match crate::direct::DirectTarget::new(&self.ctx) {
                Ok(d) => self.direct = Some(d),
                Err(e) => {
                    console::error_once(&format!(
                        "[luxel] direct target: {e}; drawing to a canvas without depth"
                    ));
                    return false;
                }
            }
        }
        let Some(d) = &mut self.direct else {
            return false;
        };
        match d.prepare(&self.ctx, w, h, self.samples, pass) {
            Ok(()) => true,
            Err(e) => {
                console::error_once(&format!(
                    "[luxel] direct target: {e}; drawing to a canvas without depth"
                ));
                false
            }
        }
    }

    // TAA off for good (`taa()` answers false); HDR output stays.
    fn give_up_taa(&mut self, e: &Error) {
        // Lost: everything fails until the restore, which rebuilds it.
        if self.ctx.is_lost() {
            return;
        }
        console::warn(&format!(
            "[luxel] TAA unavailable ({e}); HDR output without it"
        ));
        self.taa = false;
        self.taa_previous = None;
        self.renderer.set_motion_vectors(&self.ctx, false);
    }

    // Auto exposure off for good (`autoExposure()` answers false).
    fn give_up_auto_exposure(&mut self, e: &Error) {
        if self.ctx.is_lost() {
            return;
        }
        console::warn(&format!(
            "[luxel] auto exposure unavailable ({e}); the set exposure only"
        ));
        self.auto_exposure = false;
    }

    // FXAA off for good (`fxaa()` answers false).
    fn give_up_fxaa(&mut self, e: &Error) {
        if self.ctx.is_lost() {
            return;
        }
        console::warn(&format!(
            "[luxel] FXAA unavailable ({e}); output without it"
        ));
        self.fxaa = false;
    }

    fn give_up_hdr(&mut self, e: &Error) {
        if self.ctx.is_lost() {
            return;
        }
        console::warn(&format!(
            "[luxel] HDR output unavailable ({e}); drawing straight to the canvas"
        ));
        self.hdr_wanted = false;
        if let Some(out) = self.hdr.take() {
            out.destroy(&self.ctx);
        }
    }
}

impl Scene {
    // Ambient occlusion over the opaque scene in the HDR target (after the
    // opaque pass): its passes made on first use, or again after a context
    // loss.
    pub(super) fn apply_occlusion(&mut self, camera: luxel_ecs::Entity) {
        let ao = self.ambient_occlusion;
        // Contact shadows: of the key light, in the camera's view (where it
        // is, or the way toward it).
        let contact = self.contact_shadows.and_then(|c| {
            let view = self
                .world
                .get::<crate::transform::WorldMatrix>(camera)?
                .0
                .inverse();
            let (_, at) = self.renderer.key_light(&self.world)?;
            Some((c, view * at))
        });
        if ao.is_none() && contact.is_none() {
            // Both off (or nothing to apply them to): their passes and
            // targets go, made again when one comes back.
            if self.ambient_occlusion.is_none()
                && self.contact_shadows.is_none()
                && let Some(o) = self.occlusion.take()
            {
                o.destroy(&self.ctx);
            }
            return;
        }
        let Some(out) = self.hdr.as_mut() else {
            return;
        };
        if self
            .occlusion
            .as_ref()
            .is_some_and(|o| o.is_lost(&self.ctx))
            && let Some(old) = self.occlusion.take()
        {
            old.destroy(&self.ctx);
        }
        if self.occlusion.is_none() {
            match crate::occlusion::Occlusion::new(&self.ctx) {
                Ok(o) => self.occlusion = Some(o),
                Err(e) => {
                    // Both off for good: their passes are one (else they'd
                    // be compiled again every frame).
                    if !self.ctx.is_lost() {
                        console::error_once(&format!("[luxel] ambient occlusion: {e}"));
                        self.ambient_occlusion = None;
                        self.contact_shadows = None;
                    }
                    return;
                }
            }
        }
        let Some(depth) = out.screen_depth(&self.ctx) else {
            return;
        };
        let (_, size) = out.draw_target();
        let (target, key) = out.occlusion_target();
        // The scene's size (the shadow casters' box), for the auto lengths.
        let scene_size = self
            .renderer
            .caster_bounds()
            .map_or(2.0, |(lo, hi)| (hi - lo).length().max(1e-3));
        let proj = self.renderer.drawn_projection();
        if let Some(o) = self.occlusion.as_mut() {
            o.render(
                &self.ctx,
                ao.as_ref(),
                contact.as_ref().map(|(c, l)| (c, *l)),
                depth,
                target,
                key,
                size,
                &proj,
                scene_size,
                self.taa_frame,
            );
        }
        out.resolve_again(&self.ctx);
    }
}
