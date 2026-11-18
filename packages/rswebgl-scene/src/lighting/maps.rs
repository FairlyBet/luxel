//! The shadow maps' GPU side: one depth-texture array (a layer per
//! shadowing light), a framebuffer, frame block and pass per layer. The
//! lit shaders compare against it with hardware PCF filtered 3×3
//! (`light_shadow` in shadows.glsl, after Filament). Biases as Filament's:
//! the casters' depth pushed back by a polygon offset (slope and constant),
//! the receiver moved along its normal by a fraction of a texel.

use glam::Mat4;
use rswebgl::buffer::BufferUsage;
use rswebgl::context::Context;
use rswebgl::draw::Viewport;
use rswebgl::error::Error;
use rswebgl::framebuffer::ClearMask;
use rswebgl::ids::{
    FramebufferId, ParamGroupId, ParamLayoutId, StateId, TextureId, UniformBufferId,
};
use rswebgl::pass::Pass;
use rswebgl::render_state::{DepthFunc, RenderState};
use rswebgl::texture::{
    TextureCompareMode, TextureFormat, TextureMagFilter, TextureMinFilter, TextureTarget,
    TextureWrap,
};
use rswebgl::uniform_buffer::{UboLayout, UniformType};

use super::settings::LightShadow;
use super::views::ShadowView;
use crate::render::FRAME_BLOCK;

/// Layers of the shadow maps' depth array: a spot light takes one, a
/// directional light one per cascade, a point light's six faces fill a
/// quarter each (lights past them shine unshadowed).
pub const MAX_SHADOW_LAYERS: usize = 8;
/// Shadow maps (views) a frame draws: a layer or a quarter of one each.
pub const MAX_SHADOW_VIEWS: usize = 16;

/// What a view's region holds: the signature drawn, and where.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Held {
    signature: u64,
    layer: usize,
    // Texture coordinates' bits (exact compare).
    tile: [u32; 4],
}

impl Held {
    fn overlaps(&self, other: &Held) -> bool {
        let [a0, a1, a2, a3] = self.tile.map(f32::from_bits);
        let [b0, b1, b2, b3] = other.tile.map(f32::from_bits);
        self.layer == other.layer && a0 < b2 && b0 < a2 && a1 < b3 && b1 < a3
    }
}

/// The GPU side: the depth array, a framebuffer per layer, and a frame
/// block and pass per view (a layer, or a point light face's quarter).
pub(crate) struct ShadowMaps {
    /// Side of each layer (1 until a light needs them), how many, and
    /// whether their depth is 16-bit (else 24).
    size: u32,
    layers: usize,
    depth16: bool,
    texture: TextureId,
    fbs: Vec<FramebufferId>,
    ubos: Vec<UniformBufferId>,
    groups: Vec<ParamGroupId>,
    pub passes: Vec<Pass>,
    /// This frame's views.
    pub views: Vec<ShadowView>,
    // The signature (`cache`) of what each view holds (None: nothing
    // yet), and whether it is drawn again this frame.
    // Held with where it was drawn (layer, tile): the same signature in
    // another place isn't the same map.
    drawn: Vec<Option<Held>>,
    dirty: Vec<bool>,
    // Whether the comparison is set for reverse-Z.
    reverse_z: bool,
}

impl ShadowMaps {
    /// Empty maps (a 1×1 layer) and `frame_layout` groups: a frame block
    /// of their own, `shared` blocks (name, buffer) as the scene's.
    pub fn new(
        ctx: &Context,
        frame_layout: ParamLayoutId,
        shared: &[(&str, UniformBufferId)],
    ) -> Result<Self, Error> {
        let mut undo = rswebgl::undo::Undo::new(ctx);
        let mut ubos = Vec::new();
        let mut groups = Vec::new();
        for _ in 0..MAX_SHADOW_VIEWS {
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
            for &(name, block) in shared {
                ctx.param_set_block_by_name(group, name, block);
            }
            ubos.push(ubo);
            groups.push(group);
        }
        let mut maps = Self {
            size: 0,
            layers: 0,
            depth16: false,
            texture: TextureId::NULL,
            fbs: Vec::new(),
            ubos,
            groups,
            passes: (0..MAX_SHADOW_VIEWS).map(|_| Pass::to_default()).collect(),
            views: Vec::new(),
            drawn: Vec::new(),
            dirty: Vec::new(),
            reverse_z: false,
        };
        // Until a light needs them: one 1×1 layer for the lit shaders to bind.
        maps.resize(ctx, 1, 1, false)?;
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

    pub fn depth16(&self) -> bool {
        self.depth16
    }

    /// The frame group of view `i`'s pass.
    pub fn group(&self, i: usize) -> ParamGroupId {
        self.groups[i]
    }

    /// `layers` layers `size`², 16- or 24-bit (a new texture: rebind it).
    /// Errors leave the old.
    pub fn resize(
        &mut self,
        ctx: &Context,
        size: u32,
        layers: usize,
        depth16: bool,
    ) -> Result<(), Error> {
        let layers = layers.clamp(1, MAX_SHADOW_LAYERS);
        if size == self.size && layers == self.layers && depth16 == self.depth16 {
            return Ok(());
        }
        let format = if depth16 {
            TextureFormat::depth16()
        } else {
            TextureFormat::depth24()
        };
        let side = size as i32;
        // Linear: the comparison is filtered over 2×2 texels (hardware PCF).
        let texture = ctx.create_texture(
            TextureTarget::Texture2DArray,
            TextureMinFilter::Linear,
            TextureMagFilter::Linear,
        )?;
        ctx.texture_storage_3d(texture, 1, &format, side, side, layers as i32);
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
        self.depth16 = depth16;
        self.layers = layers;
        self.reverse_z = ctx.reverse_z();
        self.drawn = vec![None; MAX_SHADOW_VIEWS];
        self.dirty.clear();
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
        self.dirty.clear();
        for pass in &mut self.passes {
            pass.clear_draws();
        }
    }

    /// Which of this frame's layers to draw: those whose signature differs
    /// from what they hold (which they then hold).
    pub fn mark(&mut self, signatures: &[u64]) {
        self.dirty.clear();
        for (i, &sig) in signatures.iter().enumerate() {
            let Some(view) = self.views.get(i) else {
                continue;
            };
            let now = Held {
                signature: sig,
                layer: view.layer,
                tile: view.tile.map(f32::to_bits),
            };
            let held = self.drawn.get_mut(i);
            let stale = held.as_deref() != Some(&Some(now));
            if let Some(h) = held {
                *h = Some(now);
            }
            self.dirty.push(stale);
        }
        // What a map drawn now covers no longer holds another (one absent
        // this frame, a hidden light's): it is drawn again when it's back.
        for i in 0..self.dirty.len() {
            if !self.dirty[i] {
                continue;
            }
            let Some(Some(drawn)) = self.drawn.get(i).copied() else {
                continue;
            };
            for (j, other) in self.drawn.iter_mut().enumerate() {
                if j != i && other.is_some_and(|o| o.overlaps(&drawn)) {
                    *other = None;
                }
            }
        }
    }

    /// View `i` doesn't hold what its signature says (a caster couldn't be
    /// drawn into it): drawn again next frame.
    pub fn forget(&mut self, i: usize) {
        if let Some(h) = self.drawn.get_mut(i) {
            *h = None;
        }
    }

    /// Keeps every layer as it is this frame (nothing drawn).
    pub fn hold(&mut self) {
        self.dirty.clear();
        for pass in &mut self.passes {
            pass.clear_draws();
        }
    }

    /// Whether layer `i` is drawn this frame.
    pub fn is_dirty(&self, i: usize) -> bool {
        self.dirty.get(i).copied().unwrap_or(false)
    }

    /// Layers drawn this frame.
    pub fn redrawn(&self) -> usize {
        self.dirty.iter().filter(|d| **d).count()
    }

    /// View `i` drawn from `view`: its frame block written, its pass
    /// aimed at its layer's tile and that cleared.
    pub fn prepare(&mut self, ctx: &Context, i: usize, view: &ShadowView) {
        let ubo = self.ubos[i];
        ctx.uniform_buffer_set_mat(ubo, "u_view", &Mat4::IDENTITY.to_cols_array());
        ctx.uniform_buffer_set_mat(ubo, "u_proj", &view.view_proj.to_cols_array());
        ctx.uniform_buffer_set_mat(ubo, "u_view_proj", &view.view_proj.to_cols_array());
        ctx.uniform_buffer_set_vec4(ubo, "u_camera_pos", 0.0, 0.0, 0.0, 1.0);
        let pass = &mut self.passes[i];
        pass.set_target(self.fbs[view.layer]);
        let side = self.size as f32;
        let [x0, y0, x1, y1] = view.tile.map(|t| (t * side).round() as i32);
        let rect = Viewport::new(x0, y0, x1 - x0, y1 - y0);
        pass.set_viewport(&rect);
        pass.set_clear(&ClearMask::depth());
        pass.set_clear_rect(&rect);
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

    /// Renders this frame's layers that changed.
    pub fn render(&self, ctx: &Context) {
        for (i, pass) in self.passes[..self.views.len()].iter().enumerate() {
            if self.is_dirty(i) {
                ctx.render(pass);
            }
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
