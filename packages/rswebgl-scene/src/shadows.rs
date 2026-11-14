//! Shadow maps for directional and spot lights: each shadow-casting light
//! renders the casters' depth from its point of view into a layer of one
//! depth-texture array, which the lit shaders compare against with hardware
//! PCF filtered 3×3 (`light_shadow` in lights.glsl, after Filament).
//!
//! The shadow camera is fitted to the casters' bounds (all of them, not the
//! view): stable as the camera moves, sharp for a model, coarse for a large
//! scene (cascades are for later). Biases as Filament's: the casters'
//! depth pushed back by a polygon offset (slope and constant), the
//! receiver moved along its normal by a fraction of a shadow-map texel.

use glam::{Mat4, Vec3, Vec4};
use rswebgl::buffer::BufferUsage;
use rswebgl::context::Context;
use rswebgl::draw::Viewport;
use rswebgl::error::Error;
use rswebgl::framebuffer::ClearMask;
use rswebgl::ids::{
    FramebufferId, ParamGroupId, ParamLayoutId, ProgramId, StateId, TextureId, UniformBufferId,
};
use rswebgl::pass::Pass;
use rswebgl::render_state::{DepthFunc, RenderState};
use rswebgl::texture::{
    TextureCompareMode, TextureFormat, TextureMagFilter, TextureMinFilter, TextureTarget,
    TextureWrap,
};
use rswebgl::uniform_buffer::{UboLayout, UniformType};

use crate::light::{Light, LightKind};
use crate::render::{FRAME_BLOCK, LIGHTING_BLOCK};

/// Shadow-casting lights a frame shades with (more are left unshadowed).
pub const MAX_SHADOWS: usize = 4;

/// Makes the light on the entity (directional or spot) cast shadows.
/// Biases as Filament's `ShadowOptions` (their defaults): the casters'
/// depth pushed back by `slope_bias` × its slope + `constant_bias` depth
/// units (polygon offset), the receiver moved along its normal by
/// `normal_bias` shadow-map texels. Raise them for shadow acne (stripes on
/// lit surfaces), lower them for peter-panning (shadows coming loose).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LightShadow {
    pub normal_bias: f32,
    pub slope_bias: f32,
    pub constant_bias: f32,
}

impl Default for LightShadow {
    fn default() -> Self {
        Self {
            normal_bias: 1.0,
            slope_bias: 2.0,
            constant_bias: 0.5,
        }
    }
}

/// Draws the entity's mesh into shadow maps with `program`: a depth-only
/// shader of the same vertex stage (the PBR's `DEPTH_ONLY` variant for
/// models; the scene sets it). No component: the mesh casts no shadow.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShadowCaster {
    pub program: ProgramId,
}

/// One shadowing light's camera this frame.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ShadowView {
    /// Clip space of the shadow pass.
    pub view_proj: Mat4,
    /// World → the map's [0, 1]³ (texture coordinates, depth as stored).
    pub to_map: Mat4,
    /// The receiver's normal offset: world units, or per unit of depth
    /// along the light (`perspective`).
    pub normal_bias: f32,
    pub perspective: bool,
    pub state: LightShadow,
}

/// `light`'s shadow camera (its world matrix `world`), fitted around the
/// casters' world box `bounds`, for a `size`² map. None when nothing can
/// be in its shadow (no casters in front of a spot, a point light).
pub(crate) fn shadow_view(
    light: &Light,
    world: &Mat4,
    bounds: (Vec3, Vec3),
    size: u32,
    reverse_z: bool,
    settings: LightShadow,
) -> Option<ShadowView> {
    let (min, max) = bounds;
    if !(min.is_finite() && max.is_finite()) || min.cmpgt(max).any() {
        return None;
    }
    let corners: [Vec3; 8] = std::array::from_fn(|i| {
        Vec3::new(
            if i & 1 == 0 { min.x } else { max.x },
            if i & 2 == 0 { min.y } else { max.y },
            if i & 4 == 0 { min.z } else { max.z },
        )
    });
    let dir = (-world.z_axis.truncate()).normalize_or(Vec3::NEG_Z);
    let up = if dir.y.abs() > 0.99 { Vec3::X } else { Vec3::Y };
    let texels = size.max(1) as f32;
    let (view, proj, normal_bias, perspective) = match light.kind {
        LightKind::Directional => {
            let center = (min + max) * 0.5;
            let radius = ((max - min).length() * 0.5).max(1e-4);
            let eye = center - dir * radius * 2.0;
            let view = Mat4::look_at_rh(eye, center, up);
            let (mut lo, mut hi) = (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY));
            for c in corners {
                let p = view.transform_point3(c);
                lo = lo.min(p);
                hi = hi.max(p);
            }
            // A margin so the outermost texels still filter inside.
            let pad = (hi - lo).max_element() * 0.02 + 1e-4;
            let (l, r, b, t) = (lo.x - pad, hi.x + pad, lo.y - pad, hi.y + pad);
            let (near, far) = ((-hi.z - pad).max(1e-4), -lo.z + pad);
            let proj = if reverse_z {
                Mat4::orthographic_rh(l, r, b, t, far, near)
            } else {
                Mat4::orthographic_rh_gl(l, r, b, t, near, far)
            };
            let texel = (r - l).max(t - b) / texels;
            (view, proj, settings.normal_bias * texel, false)
        }
        LightKind::Spot { outer_cone, .. } => {
            let eye = world.w_axis.truncate();
            let view = Mat4::look_at_rh(eye, eye + dir, up);
            let depths = corners.map(|c| -view.transform_point3(c).z);
            let mut far = depths.iter().copied().fold(f32::NEG_INFINITY, f32::max);
            if far <= 0.0 {
                return None; // every caster behind the light
            }
            if light.range > 0.0 {
                far = far.min(light.range);
            }
            far *= 1.01;
            let nearest = depths.iter().copied().fold(f32::INFINITY, f32::min);
            let near = (nearest * 0.99).max(far * 1e-3);
            let fov = (2.0 * outer_cone + 0.02).min(std::f32::consts::PI - 0.05);
            let proj = if reverse_z {
                Mat4::perspective_rh(fov, 1.0, far, near)
            } else {
                Mat4::perspective_rh_gl(fov, 1.0, near, far)
            };
            // A texel's width one unit of depth away.
            let texel = 2.0 * (fov * 0.5).tan() / texels;
            (view, proj, settings.normal_bias * texel, true)
        }
        LightKind::Point => return None,
    };
    let view_proj = proj * view;
    // Clip → texture coordinates and stored depth: x, y from [−1, 1];
    // z from [−1, 1] too, or as it is under reverse-Z (zero-to-one).
    let z = if reverse_z { (1.0, 0.0) } else { (0.5, 0.5) };
    let window = Mat4::from_cols(
        Vec4::new(0.5, 0.0, 0.0, 0.0),
        Vec4::new(0.0, 0.5, 0.0, 0.0),
        Vec4::new(0.0, 0.0, z.0, 0.0),
        Vec4::new(0.5, 0.5, z.1, 1.0),
    );
    Some(ShadowView {
        view_proj,
        to_map: window * view_proj,
        normal_bias,
        perspective,
        state: settings,
    })
}

/// The GPU side: the depth array (one layer per shadowing light), a
/// framebuffer, frame block and pass per layer.
pub(crate) struct ShadowMaps {
    /// Side of each layer (1 until a light needs them), and how many.
    size: u32,
    layers: usize,
    texture: TextureId,
    fbs: Vec<FramebufferId>,
    ubos: Vec<UniformBufferId>,
    groups: Vec<ParamGroupId>,
    pub passes: Vec<Pass>,
    /// This frame's shadowing lights, by layer.
    pub views: Vec<ShadowView>,
    // Whether the comparison is set for reverse-Z.
    reverse_z: bool,
}

impl ShadowMaps {
    /// Empty maps (a 1×1 layer) and `frame_layout` groups that share
    /// the lighting block.
    pub fn new(
        ctx: &Context,
        frame_layout: ParamLayoutId,
        lighting_ubo: UniformBufferId,
    ) -> Result<Self, Error> {
        let mut undo = rswebgl::undo::Undo::new(ctx);
        let mut ubos = Vec::new();
        let mut groups = Vec::new();
        for _ in 0..MAX_SHADOWS {
            let ubo = ctx.create_uniform_buffer(
                &UboLayout::new()
                    .field("u_view", UniformType::Mat4)
                    .field("u_proj", UniformType::Mat4)
                    .field("u_view_proj", UniformType::Mat4)
                    .field("u_camera_pos", UniformType::Vec4),
                BufferUsage::DynamicDraw,
            )?;
            undo.push(move |c| c.destroy_uniform_buffer(ubo));
            let group = ctx.create_param_group(frame_layout)?;
            undo.push(move |c| c.destroy_param_group(group));
            ctx.param_set_block_by_name(group, FRAME_BLOCK, ubo);
            ctx.param_set_block_by_name(group, LIGHTING_BLOCK, lighting_ubo);
            ubos.push(ubo);
            groups.push(group);
        }
        let mut maps = Self {
            size: 0,
            layers: 0,
            texture: TextureId::NULL,
            fbs: Vec::new(),
            ubos,
            groups,
            passes: (0..MAX_SHADOWS).map(|_| Pass::to_default()).collect(),
            views: Vec::new(),
            reverse_z: false,
        };
        // Until a light needs them: one 1×1 layer for the lit shaders to bind.
        maps.resize(ctx, 1, 1)?;
        undo.keep();
        Ok(maps)
    }

    /// The depth array the lit shaders sample (`u_shadow_map`).
    pub fn texture(&self) -> TextureId {
        self.texture
    }

    pub fn size(&self) -> u32 {
        self.size
    }

    pub fn layers(&self) -> usize {
        self.layers
    }

    /// The frame group of layer `i`'s pass.
    pub fn group(&self, i: usize) -> ParamGroupId {
        self.groups[i]
    }

    /// `layers` layers `size`² (a new texture: rebind it). Errors leave the
    /// old.
    pub fn resize(&mut self, ctx: &Context, size: u32, layers: usize) -> Result<(), Error> {
        let layers = layers.clamp(1, MAX_SHADOWS);
        if size == self.size && layers == self.layers {
            return Ok(());
        }
        let side = size as i32;
        // Linear: the comparison is filtered over 2×2 texels (hardware PCF).
        let texture = ctx.create_texture(
            TextureTarget::Texture2DArray,
            TextureMinFilter::Linear,
            TextureMagFilter::Linear,
        )?;
        ctx.texture_storage_3d(
            texture,
            1,
            &TextureFormat::depth24(),
            side,
            side,
            layers as i32,
        );
        ctx.texture_set_wrap_s(texture, TextureWrap::ClampToEdge);
        ctx.texture_set_wrap_t(texture, TextureWrap::ClampToEdge);
        ctx.texture_set_compare_mode(texture, TextureCompareMode::CompareRefToTexture);
        ctx.texture_set_compare_func(texture, compare_func(ctx.reverse_z()));
        let mut fbs = Vec::new();
        for layer in 0..layers {
            match ctx.create_framebuffer(side, side) {
                Ok(fb) => {
                    ctx.framebuffer_attach_depth_layer(fb, texture, 0, layer as i32);
                    fbs.push(fb);
                }
                Err(e) => {
                    for fb in fbs {
                        ctx.destroy_framebuffer(fb);
                    }
                    ctx.destroy_texture(texture);
                    return Err(e);
                }
            }
        }
        self.free_maps(ctx);
        self.texture = texture;
        self.fbs = fbs;
        self.size = size;
        self.layers = layers;
        self.reverse_z = ctx.reverse_z();
        // Every layer cleared to the far plane once: a layer no light draws
        // into this frame (the placeholder, a hidden light's) reads as lit.
        for &fb in &self.fbs {
            let mut pass = Pass::new(fb);
            pass.set_viewport(&Viewport::new(0, 0, side, side));
            pass.set_clear(&ClearMask::depth());
            ctx.render(&pass);
        }
        Ok(())
    }

    /// A new frame: the comparison follows reverse-Z (nearer is greater
    /// then), the passes start empty.
    pub fn begin(&mut self, ctx: &Context) {
        let reverse = ctx.reverse_z();
        if reverse != self.reverse_z {
            ctx.texture_set_compare_func(self.texture, compare_func(reverse));
            self.reverse_z = reverse;
        }
        self.views.clear();
        for pass in &mut self.passes {
            pass.clear_draws();
        }
    }

    /// Layer `i` drawn from `view`: its frame block written, its pass
    /// aimed at the layer and cleared.
    pub fn prepare(&mut self, ctx: &Context, i: usize, view: &ShadowView) {
        let ubo = self.ubos[i];
        ctx.ubo_set_mat(ubo, "u_view", &Mat4::IDENTITY.to_cols_array());
        ctx.ubo_set_mat(ubo, "u_proj", &view.view_proj.to_cols_array());
        ctx.ubo_set_mat(ubo, "u_view_proj", &view.view_proj.to_cols_array());
        ctx.ubo_set_vec4(ubo, "u_camera_pos", 0.0, 0.0, 0.0, 1.0);
        let pass = &mut self.passes[i];
        pass.set_target(self.fbs[i]);
        let side = self.size as i32;
        pass.set_viewport(&Viewport::new(0, 0, side, side));
        pass.set_clear(&ClearMask::depth());
    }

    /// The casters' render state for `light`'s biases: depth only, both
    /// faces (closed meshes shadow from their front; open ones, leaves and
    /// planes, from either side), pushed back by the polygon offset.
    pub fn state(ctx: &Context, light: &LightShadow) -> StateId {
        let mut s = RenderState::new();
        // Depth is written only with the test on.
        s.depth_test = true;
        s.depth_func = DepthFunc::Less;
        s.cull_face = false;
        s.polygon_offset_fill = true;
        // Away from the light: larger depth, or smaller under reverse-Z.
        let sign = if ctx.reverse_z() { -1.0 } else { 1.0 };
        s.polygon_offset_factor = sign * light.slope_bias;
        s.polygon_offset_units = sign * light.constant_bias;
        ctx.render_state(&s)
    }

    /// Renders this frame's layers.
    pub fn render(&self, ctx: &Context) {
        for pass in &self.passes[..self.views.len()] {
            ctx.render(pass);
        }
    }

    fn free_maps(&mut self, ctx: &Context) {
        for fb in self.fbs.drain(..) {
            ctx.destroy_framebuffer(fb);
        }
        if !self.texture.is_null() {
            ctx.destroy_texture(std::mem::replace(&mut self.texture, TextureId::NULL));
        }
    }

    /// Frees it all; after a context loss (`lost`) only the CPU-side groups.
    pub fn destroy(mut self, ctx: &Context, lost: bool) {
        for g in self.groups.drain(..) {
            ctx.destroy_param_group(g);
        }
        if lost {
            return;
        }
        self.free_maps(ctx);
        for u in self.ubos.drain(..) {
            ctx.destroy_uniform_buffer(u);
        }
    }
}

// Lit when the receiver is as near the light as the caster stored, or
// nearer: smaller depth, or larger under reverse-Z.
fn compare_func(reverse_z: bool) -> DepthFunc {
    if reverse_z {
        DepthFunc::GreaterEqual
    } else {
        DepthFunc::LessEqual
    }
}
