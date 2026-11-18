use crate::error::{Error, err};
use std::collections::BTreeSet;

use wasm_bindgen::prelude::*;
use web_sys::WebGl2RenderingContext;

use crate::buffer::BufferUsage;
use crate::console;
use crate::context::Context;
use crate::ids::{BufferId, VertexArrayId};
use crate::limits;
use crate::vao::VertexAttr;

// ---------------------------------------------------------------------------
// AttrKind
// ---------------------------------------------------------------------------

#[wasm_bindgen]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AttrKind {
    // f32 vectors / matrices
    F32,
    F32x2,
    F32x3,
    F32x4,
    Mat2,
    Mat3,
    Mat4,

    // half-float (passed as u16 bits)
    F16,
    F16x2,
    F16x3,
    F16x4,

    // integer attrs (read as ivec/uvec in shader)
    I8,
    I8x2,
    I8x3,
    I8x4,
    U8,
    U8x2,
    U8x3,
    U8x4,
    I16,
    I16x2,
    I16x3,
    I16x4,
    U16,
    U16x2,
    U16x3,
    U16x4,
    I32,
    I32x2,
    I32x3,
    I32x4,
    U32,
    U32x2,
    U32x3,
    U32x4,

    // normalized -> float in shader
    I8Norm,
    I8x2Norm,
    I8x3Norm,
    I8x4Norm,
    U8Norm,
    U8x2Norm,
    U8x3Norm,
    U8x4Norm,
    I16Norm,
    I16x2Norm,
    I16x3Norm,
    I16x4Norm,
    U16Norm,
    U16x2Norm,
    U16x3Norm,
    U16x4Norm,
}

#[derive(Clone, Copy)]
struct AttrDesc {
    gl_type: u32,
    components: u32,   // per slot (1..=4)
    slots: u32,        // 1 for vectors, 2/3/4 for matrices
    element_size: u32, // bytes per scalar
    normalized: bool,
    integer: bool,
}

impl AttrKind {
    fn desc(self) -> AttrDesc {
        use AttrKind::*;
        use WebGl2RenderingContext as Gl;

        macro_rules! d {
            ($t:expr, $c:expr, $s:expr, $sz:expr, $n:expr, $i:expr) => {
                AttrDesc {
                    gl_type: $t,
                    components: $c,
                    slots: $s,
                    element_size: $sz,
                    normalized: $n,
                    integer: $i,
                }
            };
        }

        match self {
            F32 => d!(Gl::FLOAT, 1, 1, 4, false, false),
            F32x2 => d!(Gl::FLOAT, 2, 1, 4, false, false),
            F32x3 => d!(Gl::FLOAT, 3, 1, 4, false, false),
            F32x4 => d!(Gl::FLOAT, 4, 1, 4, false, false),
            Mat2 => d!(Gl::FLOAT, 2, 2, 4, false, false),
            Mat3 => d!(Gl::FLOAT, 3, 3, 4, false, false),
            Mat4 => d!(Gl::FLOAT, 4, 4, 4, false, false),

            F16 => d!(Gl::HALF_FLOAT, 1, 1, 2, false, false),
            F16x2 => d!(Gl::HALF_FLOAT, 2, 1, 2, false, false),
            F16x3 => d!(Gl::HALF_FLOAT, 3, 1, 2, false, false),
            F16x4 => d!(Gl::HALF_FLOAT, 4, 1, 2, false, false),

            I8 => d!(Gl::BYTE, 1, 1, 1, false, true),
            I8x2 => d!(Gl::BYTE, 2, 1, 1, false, true),
            I8x3 => d!(Gl::BYTE, 3, 1, 1, false, true),
            I8x4 => d!(Gl::BYTE, 4, 1, 1, false, true),
            U8 => d!(Gl::UNSIGNED_BYTE, 1, 1, 1, false, true),
            U8x2 => d!(Gl::UNSIGNED_BYTE, 2, 1, 1, false, true),
            U8x3 => d!(Gl::UNSIGNED_BYTE, 3, 1, 1, false, true),
            U8x4 => d!(Gl::UNSIGNED_BYTE, 4, 1, 1, false, true),
            I16 => d!(Gl::SHORT, 1, 1, 2, false, true),
            I16x2 => d!(Gl::SHORT, 2, 1, 2, false, true),
            I16x3 => d!(Gl::SHORT, 3, 1, 2, false, true),
            I16x4 => d!(Gl::SHORT, 4, 1, 2, false, true),
            U16 => d!(Gl::UNSIGNED_SHORT, 1, 1, 2, false, true),
            U16x2 => d!(Gl::UNSIGNED_SHORT, 2, 1, 2, false, true),
            U16x3 => d!(Gl::UNSIGNED_SHORT, 3, 1, 2, false, true),
            U16x4 => d!(Gl::UNSIGNED_SHORT, 4, 1, 2, false, true),
            I32 => d!(Gl::INT, 1, 1, 4, false, true),
            I32x2 => d!(Gl::INT, 2, 1, 4, false, true),
            I32x3 => d!(Gl::INT, 3, 1, 4, false, true),
            I32x4 => d!(Gl::INT, 4, 1, 4, false, true),
            U32 => d!(Gl::UNSIGNED_INT, 1, 1, 4, false, true),
            U32x2 => d!(Gl::UNSIGNED_INT, 2, 1, 4, false, true),
            U32x3 => d!(Gl::UNSIGNED_INT, 3, 1, 4, false, true),
            U32x4 => d!(Gl::UNSIGNED_INT, 4, 1, 4, false, true),

            I8Norm => d!(Gl::BYTE, 1, 1, 1, true, false),
            I8x2Norm => d!(Gl::BYTE, 2, 1, 1, true, false),
            I8x3Norm => d!(Gl::BYTE, 3, 1, 1, true, false),
            I8x4Norm => d!(Gl::BYTE, 4, 1, 1, true, false),
            U8Norm => d!(Gl::UNSIGNED_BYTE, 1, 1, 1, true, false),
            U8x2Norm => d!(Gl::UNSIGNED_BYTE, 2, 1, 1, true, false),
            U8x3Norm => d!(Gl::UNSIGNED_BYTE, 3, 1, 1, true, false),
            U8x4Norm => d!(Gl::UNSIGNED_BYTE, 4, 1, 1, true, false),
            I16Norm => d!(Gl::SHORT, 1, 1, 2, true, false),
            I16x2Norm => d!(Gl::SHORT, 2, 1, 2, true, false),
            I16x3Norm => d!(Gl::SHORT, 3, 1, 2, true, false),
            I16x4Norm => d!(Gl::SHORT, 4, 1, 2, true, false),
            U16Norm => d!(Gl::UNSIGNED_SHORT, 1, 1, 2, true, false),
            U16x2Norm => d!(Gl::UNSIGNED_SHORT, 2, 1, 2, true, false),
            U16x3Norm => d!(Gl::UNSIGNED_SHORT, 3, 1, 2, true, false),
            U16x4Norm => d!(Gl::UNSIGNED_SHORT, 4, 1, 2, true, false),
        }
    }

    fn bytes_per_vertex(self) -> u32 {
        let d = self.desc();
        d.components * d.slots * d.element_size
    }

    fn to_vertex_attr(self) -> VertexAttr {
        let d = self.desc();
        let c = d.components as u8;
        use WebGl2RenderingContext as Gl;

        if d.integer {
            match d.gl_type {
                x if x == Gl::INT => VertexAttr::int(c),
                x if x == Gl::UNSIGNED_INT => VertexAttr::uint(c),
                x if x == Gl::BYTE => VertexAttr::ibyte(c),
                x if x == Gl::UNSIGNED_BYTE => VertexAttr::iubyte(c),
                x if x == Gl::SHORT => VertexAttr::ishort(c),
                x if x == Gl::UNSIGNED_SHORT => VertexAttr::iushort(c),
                _ => unreachable!("invalid integer AttrKind descriptor"),
            }
        } else if d.normalized {
            match d.gl_type {
                x if x == Gl::BYTE => VertexAttr::normalized_byte(c),
                x if x == Gl::UNSIGNED_BYTE => VertexAttr::normalized_ubyte(c),
                x if x == Gl::SHORT => VertexAttr::normalized_short(c),
                x if x == Gl::UNSIGNED_SHORT => VertexAttr::normalized_ushort(c),
                _ => unreachable!("invalid normalized AttrKind descriptor"),
            }
        } else {
            match d.gl_type {
                x if x == Gl::FLOAT => VertexAttr::float(c),
                x if x == Gl::HALF_FLOAT => VertexAttr::half_float(c),
                _ => unreachable!("invalid float AttrKind descriptor"),
            }
        }
    }
}

// ---------------------------------------------------------------------------
// VaoBuilder
// ---------------------------------------------------------------------------

struct AttrEntry {
    index: u32,
    kind: AttrKind,
    data: Vec<u8>,
    source_gl_type: u32,
}

struct IndexEntry {
    data: Vec<u8>,
}

/// Builds a vertex array from per-attribute arrays: interleaves them into one
/// vertex buffer (plus an optional index buffer) that the vertex array owns —
/// `destroy_vertex_array` destroys them too.
#[wasm_bindgen]
pub struct VaoBuilder {
    ctx: Context,
    attrs: Vec<AttrEntry>,
    indices: Option<IndexEntry>,
    usage: BufferUsage,
    // `set_divisor` by attribute index (before or after its `add*`).
    divisors: Vec<(u32, u32)>,
}

fn bytes_of<T>(slice: &[T]) -> Vec<u8> {
    let len = std::mem::size_of_val(slice);
    let ptr = slice.as_ptr() as *const u8;
    unsafe { std::slice::from_raw_parts(ptr, len) }.to_vec()
}

#[wasm_bindgen]
impl VaoBuilder {
    #[wasm_bindgen(constructor)]
    pub fn new(ctx: &Context) -> Self {
        Self {
            ctx: ctx.clone(),
            attrs: Vec::new(),
            indices: None,
            usage: BufferUsage::StaticDraw,
            divisors: Vec::new(),
        }
    }

    /// Makes the attribute at `index` (added already) per instance: it
    /// advances once every `divisor` instances (1: each instance its own),
    /// for instanced draws. Per-instance attributes go in a buffer of their
    /// own, one per divisor (their counts differ: n instances need
    /// ceil(n / divisor) values); 0 makes it per vertex again. Before or
    /// after its `add*`.
    #[wasm_bindgen(js_name = setDivisor)]
    pub fn set_divisor(&mut self, index: u32, divisor: u32) {
        self.divisors.retain(|&(i, _)| i != index);
        self.divisors.push((index, divisor));
    }

    /// How the buffers it makes are used (default `StaticDraw`):
    /// `DynamicDraw` for data rewritten often (`bufferWrite` into
    /// `vertexArrayAttrBuffer`).
    #[wasm_bindgen(js_name = setUsage)]
    pub fn set_usage(&mut self, usage: BufferUsage) {
        self.usage = usage;
    }

    // One attribute's array, of GL type `source_gl_type`.
    fn push(&mut self, index: u32, kind: AttrKind, data: Vec<u8>, source_gl_type: u32) {
        self.attrs.push(AttrEntry {
            index,
            kind,
            data,
            source_gl_type,
        });
    }

    #[wasm_bindgen(js_name = addF32)]
    pub fn add_f32(&mut self, index: u32, kind: AttrKind, data: &[f32]) {
        self.push(index, kind, bytes_of(data), WebGl2RenderingContext::FLOAT);
    }

    #[wasm_bindgen(js_name = addF16)]
    pub fn add_f16(&mut self, index: u32, kind: AttrKind, data: &[u16]) {
        self.push(
            index,
            kind,
            bytes_of(data),
            WebGl2RenderingContext::HALF_FLOAT,
        );
    }

    #[wasm_bindgen(js_name = addI8)]
    pub fn add_i8(&mut self, index: u32, kind: AttrKind, data: &[i8]) {
        self.push(index, kind, bytes_of(data), WebGl2RenderingContext::BYTE);
    }

    #[wasm_bindgen(js_name = addU8)]
    pub fn add_u8(&mut self, index: u32, kind: AttrKind, data: &[u8]) {
        self.push(
            index,
            kind,
            data.to_vec(),
            WebGl2RenderingContext::UNSIGNED_BYTE,
        );
    }

    #[wasm_bindgen(js_name = addI16)]
    pub fn add_i16(&mut self, index: u32, kind: AttrKind, data: &[i16]) {
        self.push(index, kind, bytes_of(data), WebGl2RenderingContext::SHORT);
    }

    #[wasm_bindgen(js_name = addU16)]
    pub fn add_u16(&mut self, index: u32, kind: AttrKind, data: &[u16]) {
        self.push(
            index,
            kind,
            bytes_of(data),
            WebGl2RenderingContext::UNSIGNED_SHORT,
        );
    }

    #[wasm_bindgen(js_name = addI32)]
    pub fn add_i32(&mut self, index: u32, kind: AttrKind, data: &[i32]) {
        self.push(index, kind, bytes_of(data), WebGl2RenderingContext::INT);
    }

    #[wasm_bindgen(js_name = addU32)]
    pub fn add_u32(&mut self, index: u32, kind: AttrKind, data: &[u32]) {
        self.push(
            index,
            kind,
            bytes_of(data),
            WebGl2RenderingContext::UNSIGNED_INT,
        );
    }

    #[wasm_bindgen(js_name = addIndicesU8)]
    pub fn add_indices_u8(&mut self, data: &[u8]) {
        self.indices = Some(IndexEntry {
            data: data.to_vec(),
        });
    }

    #[wasm_bindgen(js_name = addIndicesU16)]
    pub fn add_indices_u16(&mut self, data: &[u16]) {
        self.indices = Some(IndexEntry {
            data: bytes_of(data),
        });
    }

    #[wasm_bindgen(js_name = addIndicesU32)]
    pub fn add_indices_u32(&mut self, data: &[u32]) {
        self.indices = Some(IndexEntry {
            data: bytes_of(data),
        });
    }

    /// Creates the vertex array. Consumes the builder: in JS the object is
    /// freed by this call and must not be used again.
    pub fn build(self) -> Result<VertexArrayId, Error> {
        let ctx = self.ctx.clone();
        let vao = ctx.create_vertex_array()?;
        if let Err(e) = self.build_into(vao) {
            ctx.destroy_vertex_array(vao);
            return Err(e);
        }
        Ok(vao)
    }

    /// Configures an existing vertex array. The buffers created here are owned
    /// by it and destroyed with it. Consumes the builder, like `build`.
    #[wasm_bindgen(js_name = buildInto)]
    pub fn build_into(self, vao: VertexArrayId) -> Result<(), Error> {
        // A stale vertex array: an error before any buffer is made (they
        // would leak, owned by nothing).
        if !self.ctx.vertex_array_is_valid(vao) {
            return Err(err!("VaoBuilder: the vertex array is stale or destroyed"));
        }
        // 1. Source type matches declared kind
        for e in &self.attrs {
            let expected = e.kind.desc().gl_type;
            if expected != e.source_gl_type {
                return Err(err!(
                    "VaoBuilder: attr index {} has kind {:?} (expects GL type 0x{:X}) but was added via a typed method for GL type 0x{:X}",
                    e.index,
                    e.kind,
                    expected,
                    e.source_gl_type
                ));
            }
        }

        // 2. Slots in range, no collisions (matrices span multiple slots)
        let max_slots = limits::max_vertex_attribs();
        let mut used: BTreeSet<u32> = BTreeSet::new();
        for e in &self.attrs {
            let d = e.kind.desc();
            for s in 0..d.slots {
                let slot = e.index + s;
                if slot >= max_slots {
                    return Err(err!(
                        "VaoBuilder: {:?} at index {} needs attribute slot {}, beyond \
                         MAX_VERTEX_ATTRIBS = {}",
                        e.kind,
                        e.index,
                        slot,
                        max_slots
                    ));
                }
                if !used.insert(slot) {
                    return Err(err!(
                        "VaoBuilder: attribute slot {} used by multiple entries (collision from {:?} at index {})",
                        slot,
                        e.kind,
                        e.index
                    ));
                }
            }
        }

        // 3. One interleaved buffer per divisor: per-vertex attributes (0)
        //    in one, per-instance ones in another per divisor — their
        //    counts differ (n instances need ceil(n / divisor) values).
        let ctx = &self.ctx;
        let divisor_of = |e: &AttrEntry| {
            self.divisors
                .iter()
                .find(|&&(i, _)| i == e.index)
                .map_or(0, |&(_, d)| d)
        };
        let mut divisors: Vec<u32> = self.attrs.iter().map(divisor_of).collect();
        divisors.sort_unstable();
        divisors.dedup();
        for divisor in divisors {
            let group: Vec<&AttrEntry> = self
                .attrs
                .iter()
                .filter(|e| divisor_of(e) == divisor)
                .collect();
            let what = if divisor > 0 {
                "instance values"
            } else {
                "vertices"
            };
            let (interleaved, stride, offsets) = interleave(&group, what)?;
            let vbo = ctx.create_buffer(self.usage, &interleaved)?;
            ctx.own_buffer(vao, vbo);
            // One vertex_attrib_pointer per slot for matrices.
            for (e, &off) in group.iter().zip(offsets.iter()) {
                let d = e.kind.desc();
                let slot_bytes = d.components * d.element_size;
                let mut attr = e.kind.to_vertex_attr();
                attr.stride = stride as i32;
                attr.divisor = divisor;
                for s in 0..d.slots {
                    attr.offset = (off + s * slot_bytes) as i32;
                    ctx.vertex_array_attr(vao, e.index + s, vbo, &attr);
                }
            }
        }

        // 4. Index buffer (optional)
        if let Some(idx) = &self.indices {
            let ibo = ctx.create_index_buffer(self.usage, &idx.data)?;
            ctx.own_buffer(vao, ibo);
            ctx.vertex_array_set_index_buffer(vao, ibo);
        }

        Ok(())
    }
}

// Interleaves `group`'s data (each attribute aligned to its element size,
// the stride to the largest): the bytes, the stride and each one's offset.
// Every attribute must have the same count of `what` (vertices, instances).
fn interleave(group: &[&AttrEntry], what: &str) -> Result<(Vec<u8>, u32, Vec<u32>), Error> {
    let mut count: Option<u32> = None;
    for e in group {
        let bpv = e.kind.bytes_per_vertex();
        if !e.data.len().is_multiple_of(bpv as usize) {
            return Err(err!(
                "VaoBuilder: attr index {} ({:?}) data length {} is not a multiple of its size {}",
                e.index,
                e.kind,
                e.data.len(),
                bpv
            ));
        }
        let n = (e.data.len() as u32) / bpv;
        match count {
            None => count = Some(n),
            Some(prev) if prev != n => {
                return Err(err!(
                    "VaoBuilder: attr index {} has {} {what}, expected {} (from an earlier attr)",
                    e.index,
                    n,
                    prev
                ));
            }
            _ => {}
        }
    }
    let count = count.unwrap_or(0);
    let mut running = 0u32;
    let mut max_align = 1u32;
    let mut offsets: Vec<u32> = Vec::with_capacity(group.len());
    for e in group {
        let d = e.kind.desc();
        let align = d.element_size;
        max_align = max_align.max(align);
        let aligned = running.next_multiple_of(align);
        if aligned != running {
            console::warn(&format!(
                "[rswebgl] VaoBuilder: padded attr at index {} from offset {} to {} for {}-byte alignment",
                e.index, running, aligned, align
            ));
        }
        offsets.push(aligned);
        running = aligned + e.kind.bytes_per_vertex();
    }
    let stride = running.next_multiple_of(max_align);
    let mut interleaved = vec![0u8; stride as usize * count as usize];
    for (e, &off) in group.iter().zip(offsets.iter()) {
        let pv = e.kind.bytes_per_vertex() as usize;
        for v in 0..count as usize {
            let dst = v * stride as usize + off as usize;
            let src = v * pv;
            interleaved[dst..dst + pv].copy_from_slice(&e.data[src..src + pv]);
        }
    }
    Ok((interleaved, stride, offsets))
}

impl Context {
    // Makes `buffer` destroyed together with `vao`.
    pub(crate) fn own_buffer(&self, vao: VertexArrayId, buffer: BufferId) {
        if let Some(v) = self.res().vertex_arrays.get_mut(vao.0) {
            v.owned.push(buffer);
        }
    }
}
