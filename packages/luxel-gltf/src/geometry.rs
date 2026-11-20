//! Translate glTF meshes into vertex arrays + draw commands, sharing GPU
//! buffers wherever the document shares data.
//!
//! Attribute locations follow a fixed convention the shaders agree on:
//!   0 = POSITION (vec3), 1 = NORMAL (vec3), 2 = TEXCOORD_0 (vec2),
//!   3 = TANGENT (vec4), 4 = COLOR_0 (vec4), 5 = TEXCOORD_1 (vec2),
//!   6 = JOINTS_0 (vec4 of joint indices, as floats), 7 = WEIGHTS_0 (vec4),
//!   8 = JOINTS_1, 9 = WEIGHTS_1 (eight influences).
//! Morph targets go into a texture per primitive (`MorphGpu`): the shader
//! adds their weighted deltas before skinning.
//! Which of them a primitive has is in `PrimitiveGpu::attributes` (`ATTR_*`).
//!
//! Sharing: one GPU buffer per glTF *accessor*, so primitives that reuse an
//! accessor (a multi-material mesh splitting one vertex set by index ranges,
//! say) reuse the buffer; vertex arrays only retain them (luxel-gl counts uses,
//! and a destroyed buffer still in use is freed with its last user). Each mesh
//! is built once however many nodes instance it.
//!
//! Attributes are read as floats (`accessor.rs`, whatever the component
//! type) and stored in the smallest GPU format that keeps them —
//! a quantized file's own, 16-bit normals, tangents, texture coordinates
//! and colors otherwise (`vertex_format.rs`); indices keep their source
//! width (u8 / u16 / u32) — WebGL2 draws all three.

use std::collections::HashMap;

use luxel_gl::buffer::BufferUsage;
use luxel_gl::context::Context;
use luxel_gl::draw::{DrawCommand, DrawMode, IndexType};
use luxel_gl::ids::{BufferId, VertexArrayId};
use luxel_gl::vao::VertexAttr;

use luxel_gl::ids::TextureId;
use luxel_gl::texture::{TextureFormat, TextureMagFilter, TextureMinFilter, TextureTarget};

use crate::vertex_format;
use crate::{
    ATTR_COLOR, ATTR_MORPH_NORMAL, ATTR_MORPH_POSITION, ATTR_MORPH_TANGENT, ATTR_NORMAL, ATTR_SKIN,
    ATTR_SKIN8, ATTR_TANGENT, ATTR_UV0, ATTR_UV1,
};

/// Texels per row of a morph texture; `MORPH_TEXTURE_WIDTH` in the shader.
pub const MORPH_TEXTURE_WIDTH: i32 = 2048;

/// A primitive's morph targets on the GPU: an RGBA16F texture of deltas
/// (RGBA32F past a half's range) —
/// for vertex `v`, target `t` and kind `k` (position, normal, tangent, those
/// present in that order) the texel `(v * targets + t) * kinds + k`,
/// `MORPH_TEXTURE_WIDTH` texels a row.
#[derive(Clone, Copy, Debug)]
pub struct MorphGpu {
    pub texture: TextureId,
    pub targets: u32,
}

/// What a material needs of a primitive's vertex array: the TEXCOORD sets in
/// its two UV slots, and the UV slot its normal map reads (MikkTSpace
/// tangents are made where the primitive has no TANGENT).
pub type VertexKey = ([u32; 2], Option<u32>);

/// One primitive on the GPU, plus its local-space bounds.
#[derive(Clone)]
pub struct PrimitiveGpu {
    pub vao: VertexArrayId,
    pub draw: DrawCommand,
    /// glTF material index, or -1 for the default material.
    pub material: i32,
    /// `ATTR_*` bits: the optional attributes bound.
    pub attributes: u32,
    pub aabb_min: [f32; 3],
    pub aabb_max: [f32; 3],
    pub morph: Option<MorphGpu>,
    /// KHR_materials_variants: (variant, material index) mappings.
    pub variants: Vec<(u32, i32)>,
    /// What `vao` was built for (the primitive's own material's needs).
    pub key: VertexKey,
    /// More vertex arrays over the same buffers, for the variants'
    /// materials that need others: (needs, vertex array, `ATTR_*` bits).
    pub alternates: Vec<(VertexKey, VertexArrayId, u32)>,
}

/// GPU buffers created so far, by accessor index, and the vertex arrays built
/// from them. Owned by the model and destroyed with it.
#[derive(Default)]
pub struct GeometryCache {
    vertex: HashMap<usize, (BufferId, VertexAttr)>,
    index: HashMap<usize, (BufferId, IndexType)>,
    meshes: HashMap<usize, Vec<PrimitiveGpu>>,
    // The TEXCOORD sets each material reads, in its two UV slots
    // (`material::assign_uv_slots`; [0, 1] for one not listed).
    uv_sets: Vec<[u32; 2]>,
    // Per material, the UV slot its normal map reads, if it has one.
    normal_slots: Vec<Option<u32>>,
    // MikkTSpace tangents made (by POSITION, NORMAL, TEXCOORD, indices).
    tangents: HashMap<(usize, usize, usize, Option<usize>), BufferId>,
    // Skin weights, renormalized (by WEIGHTS_0 and WEIGHTS_1 accessor).
    weights: HashMap<(usize, Option<usize>), (BufferId, Option<BufferId>)>,
}

impl GeometryCache {
    /// Every buffer created (to destroy with the model).
    pub fn buffers(&self) -> impl Iterator<Item = BufferId> + '_ {
        self.vertex
            .values()
            .map(|(b, _)| *b)
            .chain(self.index.values().map(|(b, _)| *b))
            .chain(
                self.weights
                    .values()
                    .flat_map(|(a, b)| std::iter::once(*a).chain(*b)),
            )
            .chain(self.tangents.values().copied())
    }

    /// Every vertex array created: one per primitive of each built mesh,
    /// and those for its variants' materials.
    pub fn vertex_arrays(&self) -> impl Iterator<Item = VertexArrayId> + '_ {
        self.meshes
            .values()
            .flatten()
            .flat_map(|p| std::iter::once(p.vao).chain(p.alternates.iter().map(|a| a.1)))
    }

    /// What material `m` (-1: the default) needs of a vertex array (as
    /// `Material::vertex_key` of it, once its UV slots are assigned).
    pub fn key(&self, m: i32) -> VertexKey {
        let Ok(m) = usize::try_from(m) else {
            return ([0, 1], None);
        };
        (
            self.uv_sets.get(m).copied().unwrap_or([0, 1]),
            self.normal_slots.get(m).copied().flatten(),
        )
    }

    /// Every morph texture created (to destroy with the model).
    pub fn morph_textures(&self) -> impl Iterator<Item = TextureId> + '_ {
        self.meshes
            .values()
            .flatten()
            .filter_map(|p| p.morph.map(|m| m.texture))
    }

    /// Number of distinct meshes built.
    pub fn mesh_count(&self) -> usize {
        self.meshes.len()
    }

    /// The TEXCOORD sets material `m` reads go into the UV slots as
    /// `uv_sets[m]` says (before building meshes).
    pub fn set_uv_sets(&mut self, uv_sets: Vec<[u32; 2]>) {
        self.uv_sets = uv_sets;
    }

    /// Which materials are normal-mapped, and the UV slot the map reads:
    /// their primitives without TANGENT get MikkTSpace tangents.
    pub fn set_normal_slots(&mut self, slots: Vec<Option<u32>>) {
        self.normal_slots = slots;
    }

    /// The primitives of `mesh`, built on first request.
    pub fn mesh(
        &mut self,
        ctx: &Context,
        mesh: &gltf::Mesh,
        buffers: &[Vec<u8>],
    ) -> &[PrimitiveGpu] {
        if !self.meshes.contains_key(&mesh.index()) {
            let prims = mesh
                .primitives()
                .filter_map(|p| self.build_primitive(ctx, &p, buffers))
                .collect();
            self.meshes.insert(mesh.index(), prims);
        }
        &self.meshes[&mesh.index()]
    }

    /// Build one primitive's vertex array. `None` (with a logged reason) if it
    /// has no POSITION or a GPU object can't be created.
    fn build_primitive(
        &mut self,
        ctx: &Context,
        primitive: &gltf::Primitive,
        buffers: &[Vec<u8>],
    ) -> Option<PrimitiveGpu> {
        let material = primitive.material().index().map_or(-1, |i| i as i32);
        let key = self.key(material);
        let reader = primitive.reader(|b| buffers.get(b.index()).map(|v| v.as_slice()));
        let Some(pos_accessor) = primitive.get(&gltf::Semantic::Positions) else {
            crate::fetch::warn("primitive has no POSITION; skipped".into());
            return None;
        };
        // Types the spec forbids would trap the gltf crate's readers (its
        // unreachable!): skipped with a reason instead.
        if let Err(why) = spec_types(primitive) {
            crate::fetch::warn(format!("primitive skipped: {why}"));
            return None;
        }
        let vertex_count = pos_accessor.count() as i32;
        // Normalized integer positions (KHR_mesh_quantization) have their min
        // and max in integer units: computed from the data then.
        let quantized =
            pos_accessor.normalized() && pos_accessor.data_type() != gltf::accessor::DataType::F32;
        let bounds = if quantized {
            None
        } else {
            accessor_bounds(&pos_accessor)
        };
        let (mut aabb_min, mut aabb_max) = bounds.unwrap_or_else(|| {
            // Bounds are required by the spec for POSITION; compute if absent.
            let mut min = [f32::INFINITY; 3];
            let mut max = [f32::NEG_INFINITY; 3];
            for p in crate::accessor::read_vecs::<3>(&pos_accessor, buffers)
                .into_iter()
                .flatten()
            {
                for i in 0..3 {
                    min[i] = min[i].min(p[i]);
                    max[i] = max[i].max(p[i]);
                }
            }
            (min, max)
        });

        // Morph targets move vertices by their weighted deltas: the box
        // grows by each target's, toward both sides (any mix of weights
        // 0–1 stays inside; three.js grows it the same way).
        for target in primitive.morph_targets() {
            let Some(deltas) = target
                .positions()
                .and_then(|a| crate::accessor::read_vecs::<3>(&a, buffers))
            else {
                continue;
            };
            let (mut lo, mut hi) = ([0.0f32; 3], [0.0f32; 3]);
            for d in deltas {
                for k in 0..3 {
                    if d[k].is_finite() {
                        lo[k] = lo[k].min(d[k]);
                        hi[k] = hi[k].max(d[k]);
                    }
                }
            }
            for k in 0..3 {
                aabb_min[k] += lo[k];
                aabb_max[k] += hi[k];
            }
        }

        // Nothing readable to draw (no vertices, or positions that don't
        // read): left out, or its ±∞ / NaN box would spoil the model's.
        let finite_box = (0..3).all(|k| {
            aabb_min[k].is_finite() && aabb_max[k].is_finite() && aabb_min[k] <= aabb_max[k]
        });
        if vertex_count == 0 || !finite_box {
            crate::fetch::warn("primitive skipped: no readable POSITION".into());
            return None;
        }

        let vao = match ctx.create_vertex_array() {
            Ok(v) => v,
            Err(e) => {
                crate::fetch::warn(format!("vertex array: {e}"));
                return None;
            }
        };

        let mut attributes = self.bind_attributes(ctx, primitive, buffers, &reader, vao, key);
        let morph = morph_texture(ctx, primitive, buffers, vertex_count as usize, attributes);
        if let Some((_, kinds)) = &morph {
            attributes |= kinds;
        }

        let mode = draw_mode(primitive.mode());
        let index = match primitive.indices() {
            Some(accessor) => match self.index_buffer(ctx, &reader, &accessor) {
                Some((buf, ty)) => Some((buf, ty, accessor.count() as i32)),
                None => {
                    ctx.destroy_vertex_array(vao);
                    // Made already, and not cached with the primitive.
                    if let Some((m, _)) = &morph {
                        ctx.destroy_texture(m.texture);
                    }
                    return None;
                }
            },
            None => None,
        };
        let draw = match index {
            Some((buf, ty, count)) => {
                ctx.vertex_array_set_index_buffer(vao, buf);
                DrawCommand::elements(mode, count, ty, 0)
            }
            None => DrawCommand::arrays(mode, 0, vertex_count),
        };

        // The variants' materials that need other attributes: a vertex array
        // each (over the same buffers), with the same morph kinds.
        let morph_kinds = morph.as_ref().map_or(0, |(_, k)| *k);
        let variants = variant_mappings(primitive);
        let mut alternates: Vec<(VertexKey, VertexArrayId, u32)> = Vec::new();
        for &(_, m) in &variants {
            let k = self.key(m);
            if k == key || alternates.iter().any(|a| a.0 == k) {
                continue;
            }
            let Ok(alt) = ctx.create_vertex_array() else {
                continue;
            };
            let bits = self.bind_attributes(ctx, primitive, buffers, &reader, alt, k);
            if let Some((buf, _, _)) = index {
                ctx.vertex_array_set_index_buffer(alt, buf);
            }
            alternates.push((k, alt, bits | morph_kinds));
        }

        Some(PrimitiveGpu {
            vao,
            draw,
            material,
            attributes,
            aabb_min,
            aabb_max,
            morph: morph.map(|(m, _)| m),
            variants,
            key,
            alternates,
        })
    }

    // Binds `primitive`'s attributes into `vao` as material needs `key`
    // say (which TEXCOORD sets, whether tangents are made); returns the
    // `ATTR_*` bits bound (morph targets aside).
    fn bind_attributes<'a, 's, F>(
        &mut self,
        ctx: &Context,
        primitive: &gltf::Primitive,
        buffers: &[Vec<u8>],
        reader: &gltf::mesh::Reader<'a, 's, F>,
        vao: VertexArrayId,
        (uv, normal_slot): VertexKey,
    ) -> u32
    where
        F: Clone + Fn(gltf::Buffer<'a>) -> Option<&'s [u8]>,
    {
        // (location, semantic, ATTR_* bit)
        let attrs = [
            (0, gltf::Semantic::Positions, 0),
            (1, gltf::Semantic::Normals, ATTR_NORMAL),
            (2, gltf::Semantic::TexCoords(uv[0]), ATTR_UV0),
            (3, gltf::Semantic::Tangents, ATTR_TANGENT),
            (4, gltf::Semantic::Colors(0), ATTR_COLOR),
            (5, gltf::Semantic::TexCoords(uv[1]), ATTR_UV1),
        ];
        let mut attributes = 0;
        for (loc, semantic, bit) in attrs {
            let Some(accessor) = primitive.get(&semantic) else {
                continue;
            };
            let Some((buf, attr)) = self.vertex_buffer(ctx, buffers, reader, &accessor, &semantic)
            else {
                continue;
            };
            ctx.vertex_array_attr(vao, loc, buf, &attr);
            attributes |= bit;
        }
        // A normal map without TANGENT: MikkTSpace tangents, as glTF 2.0
        // §3.7.2.1 asks (the shader's derivative frame otherwise).
        if attributes & ATTR_TANGENT == 0
            && attributes & ATTR_NORMAL != 0
            && primitive.mode() == gltf::mesh::Mode::Triangles
            && let Some(slot) = normal_slot
            && let Some(t) = self.mikktspace(ctx, primitive, buffers, uv[slot as usize & 1])
        {
            let (_, tangent) = vertex_format::pack(&[], 4, vertex_format::direction());
            ctx.vertex_array_attr(vao, 3, t, &tangent);
            attributes |= ATTR_TANGENT;
        }
        // Skinning needs both: four joint indices and their weights, and
        // four more with JOINTS_1 / WEIGHTS_1.
        let joints = primitive.get(&gltf::Semantic::Joints(0));
        let weights = primitive.get(&gltf::Semantic::Weights(0));
        if let (Some(j), Some(w)) = (joints, weights) {
            let more = primitive
                .get(&gltf::Semantic::Joints(1))
                .zip(primitive.get(&gltf::Semantic::Weights(1)));
            let j = self.vertex_buffer(ctx, buffers, reader, &j, &gltf::Semantic::Joints(0));
            let j1 = more.as_ref().and_then(|(j1, _)| {
                self.vertex_buffer(ctx, buffers, reader, j1, &gltf::Semantic::Joints(1))
            });
            let w = self.skin_weights(ctx, buffers, &w, more.as_ref().map(|(_, w1)| w1));
            if let (Some(j), Some((w, w1))) = (j, w) {
                ctx.vertex_array_attr(vao, 6, j.0, &j.1);
                ctx.vertex_array_attr(vao, 7, w, &VertexAttr::float(4));
                attributes |= ATTR_SKIN;
                if let (Some(j1), Some(w1)) = (j1, w1) {
                    ctx.vertex_array_attr(vao, 8, j1.0, &j1.1);
                    ctx.vertex_array_attr(vao, 9, w1, &VertexAttr::float(4));
                    attributes |= ATTR_SKIN8;
                }
            }
        }
        attributes
    }

    // MikkTSpace tangents (bevy_mikktspace, a byte-identical port of the
    // reference) for a triangle primitive, from its positions, normals and
    // TEXCOORD `set`, per vertex: a vertex shared by faces whose tangents
    // differ keeps one (as Bevy and three.js do without welding).
    fn mikktspace(
        &mut self,
        ctx: &Context,
        primitive: &gltf::Primitive,
        buffers: &[Vec<u8>],
        set: u32,
    ) -> Option<BufferId> {
        let pos = primitive.get(&gltf::Semantic::Positions)?;
        let nor = primitive.get(&gltf::Semantic::Normals)?;
        let uv = primitive.get(&gltf::Semantic::TexCoords(set))?;
        let key = (
            pos.index(),
            nor.index(),
            uv.index(),
            primitive.indices().map(|a| a.index()),
        );
        if let Some(&b) = self.tangents.get(&key) {
            return Some(b);
        }
        let positions = crate::accessor::read_vecs::<3>(&pos, buffers)?;
        let normals = crate::accessor::read_vecs::<3>(&nor, buffers)?;
        let uvs = crate::accessor::read_vecs::<2>(&uv, buffers)?;
        let n = positions.len();
        if normals.len() != n || uvs.len() != n {
            return None;
        }
        let reader = primitive.reader(|b| buffers.get(b.index()).map(|v| v.as_slice()));
        let indices: Vec<u32> = match reader.read_indices() {
            Some(i) => i.into_u32().collect(),
            None => (0..n as u32).collect(),
        };
        if indices.iter().any(|&i| i as usize >= n) {
            return None;
        }
        let mut mesh = Mikk {
            positions: &positions,
            normals: &normals,
            uvs: &uvs,
            indices: &indices,
            out: vec![[1.0, 0.0, 0.0, 1.0]; n],
        };
        bevy_mikktspace::generate_tangents(&mut mesh).ok()?;
        let b = ctx
            .create_buffer(
                BufferUsage::StaticDraw,
                &vertex_format::pack(mesh.out.as_flattened(), 4, vertex_format::direction()).0,
            )
            .inspect_err(|e| crate::fetch::warn(format!("tangents: {e}")))
            .ok()?;
        self.tangents.insert(key, b);
        Some(b)
    }

    // The skin weights (WEIGHTS_0, and WEIGHTS_1 if given) as floats,
    // renormalized so each vertex's sum to 1 (glTF 2.0 §3.7.3.3: exporters
    // quantize; a sum off 1 scales the mesh), created on first use.
    fn skin_weights(
        &mut self,
        ctx: &Context,
        buffers: &[Vec<u8>],
        w0: &gltf::Accessor,
        w1: Option<&gltf::Accessor>,
    ) -> Option<(BufferId, Option<BufferId>)> {
        let key = (w0.index(), w1.map(|a| a.index()));
        if let Some(&b) = self.weights.get(&key) {
            return Some(b);
        }
        let mut a = crate::accessor::read_vecs::<4>(w0, buffers)?;
        let mut b = match w1 {
            Some(w1) => Some(crate::accessor::read_vecs::<4>(w1, buffers)?),
            None => None,
        };
        for (i, w0) in a.iter_mut().enumerate() {
            let extra = b
                .as_ref()
                .map_or(0.0, |b| b.get(i).map_or(0.0, |w| w.iter().sum()));
            let sum: f32 = w0.iter().sum::<f32>() + extra;
            if sum > 0.0 && (sum - 1.0).abs() > 1e-6 {
                *w0 = w0.map(|w| w / sum);
                if let Some(w) = b.as_mut().and_then(|b| b.get_mut(i)) {
                    *w = w.map(|w| w / sum);
                }
            }
        }
        let upload = |v: &[[f32; 4]]| {
            ctx.create_buffer(BufferUsage::StaticDraw, bytes_of(v.as_flattened()))
                .inspect_err(|e| crate::fetch::warn(format!("skin weights: {e}")))
                .ok()
        };
        let first = upload(&a)?;
        let second = b.as_deref().and_then(upload);
        self.weights.insert(key, (first, second));
        Some((first, second))
    }

    // The buffer for a vertex accessor, created on first use.
    fn vertex_buffer<'a, 's, F>(
        &mut self,
        ctx: &Context,
        buffers: &[Vec<u8>],
        reader: &gltf::mesh::Reader<'a, 's, F>,
        accessor: &gltf::Accessor,
        semantic: &gltf::Semantic,
    ) -> Option<(BufferId, VertexAttr)>
    where
        F: Clone + Fn(gltf::Buffer<'a>) -> Option<&'s [u8]>,
    {
        if let Some(b) = self.vertex.get(&accessor.index()) {
            return Some(b.clone());
        }
        use vertex_format::Kind;
        // Every component type read as floats (dequantized), then stored in
        // the smallest format that keeps them (vertex_format.rs).
        let (data, n, kind): (Vec<f32>, usize, Kind) = match semantic {
            gltf::Semantic::Positions => (
                crate::accessor::read_floats(accessor, buffers)?,
                3,
                Kind::Exact,
            ),
            gltf::Semantic::Normals => (
                crate::accessor::read_floats(accessor, buffers)?,
                3,
                Kind::Direction,
            ),
            gltf::Semantic::Tangents => (
                crate::accessor::read_floats(accessor, buffers)?,
                4,
                Kind::Direction,
            ),
            gltf::Semantic::TexCoords(_) => (
                crate::accessor::read_floats(accessor, buffers)?,
                2,
                Kind::Unit,
            ),
            gltf::Semantic::Colors(c) => (
                flatten(reader.read_colors(*c)?.into_rgba_f32()),
                4,
                Kind::Unit,
            ),
            // Joint indices: their own integers, read as floats (exact),
            // cast back in the shader.
            gltf::Semantic::Joints(j) => (
                flatten(reader.read_joints(*j)?.into_u16().map(|j| j.map(f32::from))),
                4,
                Kind::Exact,
            ),
            _ => return None,
        };
        let format = vertex_format::choose(
            kind,
            vertex_format::Comp::of(accessor.data_type()),
            accessor.normalized(),
            &data,
        );
        let (bytes, attr) = vertex_format::pack(&data, n, format);
        match ctx.create_buffer(BufferUsage::StaticDraw, &bytes) {
            Ok(b) => {
                self.vertex.insert(accessor.index(), (b, attr.clone()));
                Some((b, attr))
            }
            Err(e) => {
                crate::fetch::warn(format!("vertex buffer: {e}"));
                None
            }
        }
    }

    // The index buffer for an accessor, in its source width, created on first use.
    fn index_buffer<'a, 's, F>(
        &mut self,
        ctx: &Context,
        reader: &gltf::mesh::Reader<'a, 's, F>,
        accessor: &gltf::Accessor,
    ) -> Option<(BufferId, IndexType)>
    where
        F: Clone + Fn(gltf::Buffer<'a>) -> Option<&'s [u8]>,
    {
        if let Some(&b) = self.index.get(&accessor.index()) {
            return Some(b);
        }
        use gltf::mesh::util::ReadIndices;
        let (bytes, ty): (Vec<u8>, IndexType) = match reader.read_indices()? {
            ReadIndices::U8(it) => (it.collect(), IndexType::UnsignedByte),
            ReadIndices::U16(it) => (
                it.flat_map(u16::to_le_bytes).collect(),
                IndexType::UnsignedShort,
            ),
            ReadIndices::U32(it) => (
                it.flat_map(u32::to_le_bytes).collect(),
                IndexType::UnsignedInt,
            ),
        };
        match ctx.create_index_buffer(BufferUsage::StaticDraw, &bytes) {
            Ok(b) => {
                self.index.insert(accessor.index(), (b, ty));
                Some((b, ty))
            }
            Err(e) => {
                crate::fetch::warn(format!("index buffer: {e}"));
                None
            }
        }
    }
}

// The accessor's declared min/max, if it has them as 3 floats.
fn accessor_bounds(a: &gltf::Accessor) -> Option<([f32; 3], [f32; 3])> {
    let read = |v: Option<gltf::json::Value>| -> Option<[f32; 3]> {
        let arr = v?.as_array()?.clone();
        if arr.len() != 3 {
            return None;
        }
        let mut out = [0.0; 3];
        for (o, x) in out.iter_mut().zip(arr) {
            *o = x.as_f64()? as f32;
        }
        Some(out)
    };
    Some((read(a.min())?, read(a.max())?))
}

// A primitive's KHR_materials_variants mappings: (variant, material).
fn variant_mappings(primitive: &gltf::Primitive) -> Vec<(u32, i32)> {
    let Some(maps) = primitive
        .extension_value("KHR_materials_variants")
        .and_then(|v| v.get("mappings"))
        .and_then(|m| m.as_array())
    else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for m in maps {
        let Some(material) = m.get("material").and_then(|v| v.as_u64()) else {
            continue;
        };
        for v in m
            .get("variants")
            .and_then(|v| v.as_array())
            .into_iter()
            .flatten()
        {
            if let Some(v) = v.as_u64() {
                out.push((v as u32, material as i32));
            }
        }
    }
    out
}

// A triangle list for bevy_mikktspace, and the tangents it gives back.
struct Mikk<'a> {
    positions: &'a [[f32; 3]],
    normals: &'a [[f32; 3]],
    uvs: &'a [[f32; 2]],
    indices: &'a [u32],
    out: Vec<[f32; 4]>,
}

impl Mikk<'_> {
    fn vertex(&self, face: usize, vert: usize) -> usize {
        self.indices[face * 3 + vert] as usize
    }
}

impl bevy_mikktspace::Geometry for Mikk<'_> {
    fn num_faces(&self) -> usize {
        self.indices.len() / 3
    }

    fn num_vertices_of_face(&self, _face: usize) -> usize {
        3
    }

    fn position(&self, face: usize, vert: usize) -> [f32; 3] {
        self.positions[self.vertex(face, vert)]
    }

    fn normal(&self, face: usize, vert: usize) -> [f32; 3] {
        self.normals[self.vertex(face, vert)]
    }

    fn tex_coord(&self, face: usize, vert: usize) -> [f32; 2] {
        self.uvs[self.vertex(face, vert)]
    }

    fn set_tangent(
        &mut self,
        tangent_space: Option<bevy_mikktspace::TangentSpace>,
        face: usize,
        vert: usize,
    ) {
        if let Some(t) = tangent_space {
            let v = self.vertex(face, vert);
            // xyz and the bitangent's sign. glTF's v runs down the image,
            // MikkTSpace's up: w flips, as three.js'
            // computeMikkTSpaceTangents and Khronos' sample renderer do.
            let [x, y, z, w] = t.tangent_encoded();
            self.out[v] = [x, y, z, -w];
        }
    }
}

/// glTF primitive topology → our `DrawMode` (the GL enum values are identical).
fn draw_mode(mode: gltf::mesh::Mode) -> DrawMode {
    use gltf::mesh::Mode;
    match mode {
        Mode::Points => DrawMode::Points,
        Mode::Lines => DrawMode::Lines,
        Mode::LineLoop => DrawMode::LineLoop,
        Mode::LineStrip => DrawMode::LineStrip,
        Mode::Triangles => DrawMode::Triangles,
        Mode::TriangleStrip => DrawMode::TriangleStrip,
        Mode::TriangleFan => DrawMode::TriangleFan,
    }
}

/// Iterator of `[f32; N]` → flat `Vec<f32>`.
fn flatten<const N: usize>(v: impl Iterator<Item = [f32; N]>) -> Vec<f32> {
    let mut out = Vec::with_capacity(v.size_hint().0 * N);
    for a in v {
        out.extend_from_slice(&a);
    }
    out
}

// Displacements of one morph target: positions, normals, tangents.
type TargetDeltas = (
    Option<Vec<[f32; 3]>>,
    Option<Vec<[f32; 3]>>,
    Option<Vec<[f32; 3]>>,
);

// The primitive's morph targets as a texture of deltas (see `MorphGpu`), and
// the `ATTR_MORPH_*` kinds it holds. Normal and tangent deltas only where the
// primitive has normals and tangents (glTF 2.0 §3.7.2.2). None without
// targets, or when they don't fit a texture.
fn morph_texture(
    ctx: &Context,
    primitive: &gltf::Primitive,
    buffers: &[Vec<u8>],
    vertices: usize,
    attributes: u32,
) -> Option<(MorphGpu, u32)> {
    // Deltas of any component type (KHR_mesh_quantization), dequantized.
    let read =
        |a: Option<gltf::Accessor>| a.and_then(|a| crate::accessor::read_vecs::<3>(&a, buffers));
    let targets: Vec<TargetDeltas> = primitive
        .morph_targets()
        .map(|t| (read(t.positions()), read(t.normals()), read(t.tangents())))
        .collect();
    if targets.is_empty() {
        return None;
    }
    let mut kinds = 0;
    if targets.iter().any(|t| t.0.is_some()) {
        kinds |= ATTR_MORPH_POSITION;
    }
    if attributes & ATTR_NORMAL != 0 && targets.iter().any(|t| t.1.is_some()) {
        kinds |= ATTR_MORPH_NORMAL;
    }
    if attributes & ATTR_TANGENT != 0 && targets.iter().any(|t| t.2.is_some()) {
        kinds |= ATTR_MORPH_TANGENT;
    }
    if kinds == 0 {
        return None;
    }
    let per = kinds.count_ones() as usize;
    let width = MORPH_TEXTURE_WIDTH as usize;
    let rows = (vertices * targets.len() * per).div_ceil(width);
    if rows > luxel_gl::limits::max_texture_size() as usize {
        crate::fetch::warn(format!(
            "{} morph targets of {vertices} vertices don't fit a texture; \
             left out",
            targets.len()
        ));
        return None;
    }
    let mut data = vec![0.0f32; rows * width * 4];
    for v in 0..vertices {
        for (t, (p, n, tg)) in targets.iter().enumerate() {
            let base = (v * targets.len() + t) * per;
            let mut k = 0;
            for (kind, deltas) in [
                (ATTR_MORPH_POSITION, p),
                (ATTR_MORPH_NORMAL, n),
                (ATTR_MORPH_TANGENT, tg),
            ] {
                if kinds & kind == 0 {
                    continue;
                }
                if let Some(d) = deltas.as_ref().and_then(|d| d.get(v)) {
                    data[(base + k) * 4..][..3].copy_from_slice(d);
                }
                k += 1;
            }
        }
    }
    let texture = match ctx.create_texture(
        TextureTarget::Texture2D,
        TextureMinFilter::Nearest,
        TextureMagFilter::Nearest,
    ) {
        Ok(t) => t,
        Err(e) => {
            crate::fetch::warn(format!("morph texture: {e}"));
            return None;
        }
    };
    // Half precision (half the memory; a delta keeps 11 significant bits),
    // unless a delta goes past what a half holds.
    let format = if vertex_format::enabled() && data.iter().all(|d| d.abs() < 60000.0) {
        TextureFormat::rgba16f_from_float()
    } else {
        TextureFormat::rgba32f()
    };
    let (w, h) = (MORPH_TEXTURE_WIDTH, rows as i32);
    ctx.texture_storage_2d(texture, 1, &format, w, h);
    ctx.texture_sub_image_2d(texture, 0, 0, 0, w, h, &format, bytes_of(&data));
    let morph = MorphGpu {
        texture,
        targets: targets.len() as u32,
    };
    Some((morph, kinds))
}

fn bytes_of(v: &[f32]) -> &[u8] {
    // SAFETY: f32 has no padding or invalid bit patterns as bytes; the slice
    // covers exactly the same memory.
    unsafe { std::slice::from_raw_parts(v.as_ptr().cast(), std::mem::size_of_val(v)) }
}

// The component types and shapes glTF allows for what the gltf crate's
// readers read (indices, colors, joints, weights): anything else is a
// malformed file its readers don't handle (they panic).
fn spec_types(primitive: &gltf::Primitive) -> Result<(), String> {
    use gltf::Semantic;
    use gltf::accessor::{DataType as T, Dimensions as D};
    let check = |what: &str, a: &gltf::Accessor, dims: &[D], types: &[T]| {
        if dims.contains(&a.dimensions()) && types.contains(&a.data_type()) {
            Ok(())
        } else {
            Err(format!(
                "{what} as {:?} {:?} isn't allowed by glTF",
                a.dimensions(),
                a.data_type()
            ))
        }
    };
    if let Some(a) = primitive.indices() {
        check("indices", &a, &[D::Scalar], &[T::U8, T::U16, T::U32])?;
    }
    for (semantic, a) in primitive.attributes() {
        match semantic {
            Semantic::Colors(_) => {
                check("COLOR", &a, &[D::Vec3, D::Vec4], &[T::F32, T::U8, T::U16])?
            }
            Semantic::Joints(_) => check("JOINTS", &a, &[D::Vec4], &[T::U8, T::U16])?,
            Semantic::Weights(_) => check("WEIGHTS", &a, &[D::Vec4], &[T::F32, T::U8, T::U16])?,
            // (Normalized or not: KHR_mesh_quantization's integer forms too.)
            Semantic::Positions | Semantic::Normals => check(
                "POSITION / NORMAL",
                &a,
                &[D::Vec3],
                &[T::F32, T::I8, T::U8, T::I16, T::U16],
            )?,
            Semantic::Tangents => check("TANGENT", &a, &[D::Vec4], &[T::F32, T::I8, T::I16])?,
            Semantic::TexCoords(_) => check(
                "TEXCOORD",
                &a,
                &[D::Vec2],
                &[T::F32, T::I8, T::U8, T::I16, T::U16],
            )?,
            _ => {}
        }
    }
    // Every attribute has a value per vertex.
    let positions = primitive
        .get(&gltf::Semantic::Positions)
        .map_or(0, |p| p.count());
    if let Some((s, _)) = primitive.attributes().find(|(_, a)| a.count() < positions) {
        return Err(format!("{s:?} has fewer values than POSITION"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use gltf::mesh::Mode;

    #[test]
    fn primitive_mode_maps_to_draw_mode() {
        assert_eq!(draw_mode(Mode::Points), DrawMode::Points);
        assert_eq!(draw_mode(Mode::Lines), DrawMode::Lines);
        assert_eq!(draw_mode(Mode::LineLoop), DrawMode::LineLoop);
        assert_eq!(draw_mode(Mode::LineStrip), DrawMode::LineStrip);
        assert_eq!(draw_mode(Mode::Triangles), DrawMode::Triangles);
        assert_eq!(draw_mode(Mode::TriangleStrip), DrawMode::TriangleStrip);
        assert_eq!(draw_mode(Mode::TriangleFan), DrawMode::TriangleFan);
    }

    // Types glTF forbids (float indices, 2-component colors, float joints)
    // are refused up front: the gltf crate's readers would panic on them.
    #[test]
    fn forbidden_types_are_refused() {
        let doc = |indices: u32, color: &str, joints: u32| {
            let json = format!(
                r#"{{"asset":{{"version":"2.0"}},
                "buffers":[{{"byteLength":64}}],
                "bufferViews":[{{"buffer":0,"byteLength":64}}],
                "accessors":[
                  {{"bufferView":0,"componentType":5126,"count":1,"type":"VEC3","min":[0,0,0],"max":[0,0,0]}},
                  {{"bufferView":0,"componentType":{indices},"count":3,"type":"SCALAR"}},
                  {{"bufferView":0,"componentType":5126,"count":1,"type":"{color}"}},
                  {{"bufferView":0,"componentType":{joints},"count":1,"type":"VEC4"}}],
                "meshes":[{{"primitives":[{{"attributes":{{"POSITION":0,"COLOR_0":2,"JOINTS_0":3}},"indices":1}}]}}]}}"#
            );
            gltf::Gltf::from_slice_without_validation(json.as_bytes()).unwrap()
        };
        let check =
            |g: &gltf::Gltf| spec_types(&g.meshes().next().unwrap().primitives().next().unwrap());
        assert!(check(&doc(5123, "VEC4", 5121)).is_ok());
        assert!(
            check(&doc(5126, "VEC4", 5121))
                .unwrap_err()
                .contains("indices")
        );
        assert!(
            check(&doc(5123, "VEC2", 5121))
                .unwrap_err()
                .contains("COLOR")
        );
        assert!(
            check(&doc(5123, "VEC4", 5126))
                .unwrap_err()
                .contains("JOINTS")
        );
    }

    #[test]
    fn flatten_concatenates() {
        let v = [[1.0f32, 2.0, 3.0], [4.0, 5.0, 6.0]];
        assert_eq!(flatten(v.into_iter()), vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0]);
    }
}
