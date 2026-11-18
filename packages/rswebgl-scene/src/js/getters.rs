//! Getters for what the setters set: each value read back as it was set
//! (clamped where the setter clamps).

use rswebgl_ecs::Entity;
use wasm_bindgen::prelude::*;

use super::Scene;
use crate::animation::ModelAnimations;
use crate::camera::{Camera, Projection};
use crate::culling::Bounds;
use crate::lighting::{Light, LightKind, LightShadow};
use crate::render::Visible;
use crate::transform::Transform;

// A plain object from (key, value) pairs.
pub(super) fn object(pairs: &[(&str, JsValue)]) -> JsValue {
    let o = js_sys::Object::new();
    for (k, v) in pairs {
        let _ = js_sys::Reflect::set(&o, &(*k).into(), v);
    }
    o.into()
}

#[wasm_bindgen]
impl Scene {
    /// The exposure (`setExposure`), in stops.
    pub fn exposure(&self) -> f32 {
        self.renderer.exposure()
    }

    /// The environment light's intensity per channel
    /// (`setEnvironmentIntensity`).
    #[wasm_bindgen(js_name = environmentIntensity)]
    pub fn environment_intensity(&self) -> Vec<f32> {
        self.renderer.environment_intensity().to_array().to_vec()
    }

    /// The environment's turn about +Y in radians (`setEnvironmentRotation`).
    #[wasm_bindgen(js_name = environmentRotation)]
    pub fn environment_rotation(&self) -> f32 {
        self.renderer.environment_rotation()
    }

    /// The background's blur (`setBackground`, 0–1), or undefined when the
    /// background is off (`clearBackground`).
    pub fn background(&self) -> Option<f32> {
        self.renderer.background()
    }

    /// MSAA samples asked for (`setSamples`; the target may take fewer).
    pub fn samples(&self) -> i32 {
        self.samples
    }

    /// Whether the display dithers (`setDither`).
    pub fn dither(&self) -> bool {
        self.dither
    }

    /// Whether reverse-Z is on (`setReverseZ`).
    #[wasm_bindgen(js_name = reverseZ)]
    pub fn reverse_z(&self) -> bool {
        self.ctx.reverse_z()
    }

    /// Whether models are fetched again after a context loss
    /// (`setRestoreModels`).
    #[wasm_bindgen(js_name = restoreModels)]
    pub fn restore_models(&self) -> bool {
        self.restore_models
    }

    /// Auto exposure's `[grey, min, max, speed]` (`setAutoExposureOptions`).
    #[wasm_bindgen(js_name = autoExposureOptions)]
    pub fn auto_exposure_options(&self) -> Vec<f32> {
        self.auto_exposure_options.to_vec()
    }

    /// Whether cascades are tinted (`setShadowCascadeDebug`).
    #[wasm_bindgen(js_name = shadowCascadeDebug)]
    pub fn shadow_cascade_debug(&self) -> bool {
        self.renderer.debug_cascades()
    }

    /// The playback speed of `root`'s animation (`setAnimationSpeed`), or
    /// undefined when none plays.
    #[wasm_bindgen(js_name = animationSpeed)]
    pub fn animation_speed(&self, root: Entity) -> Option<f32> {
        self.world
            .get::<ModelAnimations>(root)
            .and_then(|a| a.playing.map(|p| p.speed))
    }

    /// The index of the animation `root` plays, or undefined.
    #[wasm_bindgen(js_name = playingAnimation)]
    pub fn playing_animation(&self, root: Entity) -> Option<u32> {
        self.world
            .get::<ModelAnimations>(root)
            .and_then(|a| a.playing.map(|p| p.clip as u32))
    }

    /// `e`'s camera as `setCamera` takes it (`CameraOptions`), or undefined
    /// if it isn't a camera.
    #[wasm_bindgen(unchecked_return_type = "CameraOptions | undefined")]
    pub fn camera(&self, e: Entity) -> JsValue {
        let Some(c) = self.world.get::<Camera>(e) else {
            return JsValue::UNDEFINED;
        };
        match c.projection {
            Projection::Perspective { fov_y, near, far } => object(&[
                ("type", "perspective".into()),
                ("yfov", fov_y.into()),
                ("znear", near.into()),
                ("zfar", far.into()),
            ]),
            Projection::Orthographic {
                half_height,
                near,
                far,
            } => object(&[
                ("type", "orthographic".into()),
                ("ymag", half_height.into()),
                ("znear", near.into()),
                ("zfar", far.into()),
            ]),
        }
    }

    /// `e`'s light as `setLight` takes it (`LightOptions`, shadow settings
    /// included), or undefined if it isn't a light.
    #[wasm_bindgen(unchecked_return_type = "LightOptions | undefined")]
    pub fn light(&self, e: Entity) -> JsValue {
        let Some(l) = self.world.get::<Light>(e) else {
            return JsValue::UNDEFINED;
        };
        let color = js_sys::Float32Array::from(&l.color.to_array()[..]);
        let mut pairs: Vec<(&str, JsValue)> = vec![
            ("color", color.into()),
            ("intensity", l.intensity.into()),
            ("range", l.range.into()),
        ];
        match l.kind {
            LightKind::Directional => pairs.push(("type", "directional".into())),
            LightKind::Point => pairs.push(("type", "point".into())),
            LightKind::Spot {
                inner_cone,
                outer_cone,
            } => {
                pairs.push(("type", "spot".into()));
                pairs.push(("innerConeAngle", inner_cone.into()));
                pairs.push(("outerConeAngle", outer_cone.into()));
            }
        }
        let shadow = self.world.get::<LightShadow>(e);
        pairs.push(("castShadow", shadow.is_some().into()));
        if let Some(s) = shadow {
            pairs.push(("shadowNormalBias", s.normal_bias.into()));
            pairs.push(("shadowSlopeBias", s.slope_bias.into()));
            pairs.push(("shadowConstantBias", s.constant_bias.into()));
            pairs.push(("shadowCascades", s.cascades.into()));
            pairs.push(("shadowSplitLambda", s.split_lambda.into()));
            pairs.push(("shadowMaxDistance", s.max_distance.into()));
            pairs.push(("shadowMapSize", s.map_size.into()));
        }
        object(&pairs)
    }

    /// `e`'s own visibility flag (`setVisible`; true without one). A hidden
    /// ancestor still hides it: `isShown` for that.
    pub fn visible(&self, e: Entity) -> bool {
        self.world.get::<Visible>(e).is_none_or(|v| v.0)
    }

    /// Whether `e` is drawn as far as visibility goes: it and every
    /// ancestor visible.
    #[wasm_bindgen(js_name = isShown)]
    pub fn is_shown(&self, e: Entity) -> bool {
        self.world.is_alive(e) && !crate::render::hidden(&self.world, e)
    }

    /// `e`'s local bounds (`setBounds`, or a glTF mesh's): `[minX, minY,
    /// minZ, maxX, maxY, maxZ]`, or empty without any.
    pub fn bounds(&self, e: Entity) -> Vec<f32> {
        self.world.get::<Bounds>(e).map_or_else(Vec::new, |b| {
            [b.min.to_array(), b.max.to_array()].concat()
        })
    }

    /// `e`'s local transform as a column-major 4×4 matrix (relative to its
    /// parent; `worldMatrix` for the world one), or empty without one.
    #[wasm_bindgen(js_name = localMatrix)]
    pub fn local_matrix(&self, e: Entity) -> Vec<f32> {
        self.world
            .get::<Transform>(e)
            .map_or_else(Vec::new, |t| t.matrix().to_cols_array().to_vec())
    }

    /// `e`'s children (entities whose parent it is), in no set order.
    #[wasm_bindgen(unchecked_return_type = "Handles<Entity>")]
    pub fn children(&self, e: Entity) -> Vec<u32> {
        crate::hierarchy::children(&self.world, e)
            .into_iter()
            .map(|c| c.to_raw())
            .collect()
    }

    /// The material variant `root`'s model shows (`setMaterialVariant`), or
    /// undefined for its own materials.
    #[wasm_bindgen(js_name = materialVariant)]
    pub fn material_variant(&self, root: Entity) -> Option<String> {
        self.active_variants.get(&root).cloned()
    }
}
