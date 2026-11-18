use crate::error::Error;
use luxel_core::pool::Destroyed;
use wasm_bindgen::prelude::*;
use web_sys::{WebGl2RenderingContext, WebGlVertexArrayObject};

use crate::buffer::BufferKind;
use crate::console;
use crate::context::{Context, Resources};
use crate::ids::{BufferId, VertexArrayId};
use crate::limits;

// ---------------------------------------------------------------------------
// AttrType
// ---------------------------------------------------------------------------

#[wasm_bindgen]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AttrType {
    Byte = 0x1400,          // BYTE
    UnsignedByte = 0x1401,  // UNSIGNED_BYTE
    Short = 0x1402,         // SHORT
    UnsignedShort = 0x1403, // UNSIGNED_SHORT
    Int = 0x1404,           // INT
    UnsignedInt = 0x1405,   // UNSIGNED_INT
    Float = 0x1406,         // FLOAT
    HalfFloat = 0x140B,     // HALF_FLOAT
}

impl AttrType {
    fn as_gl(&self) -> u32 {
        *self as u32
    }
}

// ---------------------------------------------------------------------------
// VertexAttr
// ---------------------------------------------------------------------------

#[wasm_bindgen]
#[derive(Clone, Debug)]
pub struct VertexAttr {
    size: u8,
    attr_type: AttrType,
    normalized: bool,
    integer: bool,
    pub stride: i32,
    pub offset: i32,
    pub divisor: u32,
}

impl VertexAttr {
    fn new(size: u8, attr_type: AttrType, normalized: bool, integer: bool) -> Self {
        Self {
            size,
            attr_type,
            normalized,
            integer,
            stride: 0,
            offset: 0,
            divisor: 0,
        }
    }
}

#[wasm_bindgen]
impl VertexAttr {
    // --- float pointer (vertexAttribPointer) ---

    pub fn float(size: u8) -> Self {
        Self::new(size, AttrType::Float, false, false)
    }
    #[wasm_bindgen(js_name = halfFloat)]
    pub fn half_float(size: u8) -> Self {
        Self::new(size, AttrType::HalfFloat, false, false)
    }
    #[wasm_bindgen(js_name = normalizedByte)]
    pub fn normalized_byte(size: u8) -> Self {
        Self::new(size, AttrType::Byte, true, false)
    }
    #[wasm_bindgen(js_name = normalizedUbyte)]
    pub fn normalized_ubyte(size: u8) -> Self {
        Self::new(size, AttrType::UnsignedByte, true, false)
    }
    #[wasm_bindgen(js_name = normalizedShort)]
    pub fn normalized_short(size: u8) -> Self {
        Self::new(size, AttrType::Short, true, false)
    }
    #[wasm_bindgen(js_name = normalizedUshort)]
    pub fn normalized_ushort(size: u8) -> Self {
        Self::new(size, AttrType::UnsignedShort, true, false)
    }
    /// Any component type read as floats: normalized (to [0, 1] / [-1, 1])
    /// or as the integer values themselves (`vertexAttribPointer`).
    #[wasm_bindgen(js_name = floatFrom)]
    pub fn float_from(size: u8, attr_type: AttrType, normalized: bool) -> Self {
        Self::new(size, attr_type, normalized, false)
    }

    // --- integer pointer (vertexAttribIPointer) ---

    pub fn int(size: u8) -> Self {
        Self::new(size, AttrType::Int, false, true)
    }
    pub fn uint(size: u8) -> Self {
        Self::new(size, AttrType::UnsignedInt, false, true)
    }
    pub fn ibyte(size: u8) -> Self {
        Self::new(size, AttrType::Byte, false, true)
    }
    pub fn iubyte(size: u8) -> Self {
        Self::new(size, AttrType::UnsignedByte, false, true)
    }
    pub fn ishort(size: u8) -> Self {
        Self::new(size, AttrType::Short, false, true)
    }
    pub fn iushort(size: u8) -> Self {
        Self::new(size, AttrType::UnsignedShort, false, true)
    }
}

// ---------------------------------------------------------------------------
// VertexArray
// ---------------------------------------------------------------------------

/// A vertex array in the context's pool. The buffers it reads are retained in
/// the buffer pool (a use count, not ownership): destroying such a buffer is
/// deferred until no vertex array uses it. Buffers in `owned` were created
/// together with the vertex array (by `VaoBuilder`) and are destroyed with it.
#[derive(Debug)]
pub(crate) struct VertexArrayObj {
    pub(crate) raw: WebGlVertexArrayObject,
    // Per attribute index: its buffer and layout (offset, stride, divisor).
    attribs: Vec<Option<(BufferId, [i32; 3])>>,
    index_buffer: Option<BufferId>,
    pub(crate) owned: Vec<BufferId>,
}

// Drops one use of a buffer; deletes it if that was the last use of a buffer
// that was already destroyed.
pub(crate) fn release_buffer(res: &mut Resources, gl: &WebGl2RenderingContext, id: BufferId) {
    if let Some(obj) = res.buffers.release(id.0) {
        obj.delete(gl);
    }
}

fn attr_index_in_range(index: u32) -> bool {
    let max = limits::max_vertex_attribs();
    if index >= max {
        console::error(&format!(
            "[luxel] vertex attribute index {index} is out of range (MAX_VERTEX_ATTRIBS = \
             {max}); ignored"
        ));
    }
    index < max
}

impl Context {
    pub(crate) fn insert_vertex_array(
        &self,
        raw: WebGlVertexArrayObject,
    ) -> Result<VertexArrayId, Error> {
        let obj = VertexArrayObj {
            raw,
            attribs: Vec::new(),
            index_buffer: None,
            owned: Vec::new(),
        };
        self.res()
            .vertex_arrays
            .insert(obj)
            .map(Into::into)
            .map_err(Error::from)
    }
}

#[wasm_bindgen]
impl Context {
    #[wasm_bindgen(js_name = createVertexArray)]
    pub fn create_vertex_array(&self) -> Result<VertexArrayId, Error> {
        self.check_alive()?;
        let raw = self
            .gl()
            .create_vertex_array()
            .ok_or("createVertexArray failed")?;
        self.insert_vertex_array(raw)
    }
    /// Whether `vao` names a live vertex array (false once destroyed, or after
    /// a context loss).
    #[wasm_bindgen(js_name = vertexArrayIsValid)]
    pub fn vertex_array_is_valid(&self, vao: VertexArrayId) -> bool {
        self.res().vertex_arrays.get(vao.0).is_some()
    }

    /// Reads attribute `index` from `buffer` (a vertex buffer). The vertex array
    /// keeps using the buffer until the attribute is replaced or removed.
    #[wasm_bindgen(js_name = vertexArrayAttr)]
    pub fn vertex_array_attr(
        &self,
        vao: VertexArrayId,
        index: u32,
        buffer: BufferId,
        attr: &VertexAttr,
    ) {
        if !attr_index_in_range(index) {
            return;
        }
        let gl = self.gl();
        let res = &mut *self.res();
        let Some(v) = res.vertex_arrays.get(vao.0) else {
            return self.stale("vertex array", "vertex_array_attr", vao);
        };
        let Some(b) = res.buffers.get(buffer.0) else {
            return self.stale("buffer", "vertex_array_attr", buffer);
        };
        if b.kind != BufferKind::Generic {
            console::error("[luxel] vertex_array_attr: an index buffer can't hold vertex data");
            return;
        }

        self.gl_state().forget_vao();
        gl.bind_vertex_array(Some(&v.raw));
        gl.bind_buffer(WebGl2RenderingContext::ARRAY_BUFFER, Some(&b.raw));
        if attr.integer {
            gl.vertex_attrib_i_pointer_with_i32(
                index,
                attr.size as i32,
                attr.attr_type.as_gl(),
                attr.stride,
                attr.offset,
            );
        } else {
            gl.vertex_attrib_pointer_with_i32(
                index,
                attr.size as i32,
                attr.attr_type.as_gl(),
                attr.normalized,
                attr.stride,
                attr.offset,
            );
        }
        gl.enable_vertex_attrib_array(index);
        gl.vertex_attrib_divisor(index, attr.divisor);
        gl.bind_buffer(WebGl2RenderingContext::ARRAY_BUFFER, None);
        gl.bind_vertex_array(None);

        res.buffers.retain(buffer.0);
        let v = res.vertex_arrays.get_mut(vao.0).expect("checked above");
        let idx = index as usize;
        if idx >= v.attribs.len() {
            v.attribs.resize(idx + 1, None);
        }
        let layout = [attr.offset, attr.stride, attr.divisor as i32];
        if let Some((prev, _)) = v.attribs[idx].replace((buffer, layout)) {
            release_buffer(res, gl, prev);
        }
    }

    /// The buffer attribute `index` of `vao` reads (NULL if none): e.g.
    /// one `VaoBuilder` made, to rewrite per-instance data with
    /// `bufferWrite` (layout: `vertexArrayAttrLayout`).
    #[wasm_bindgen(js_name = vertexArrayAttrBuffer)]
    pub fn vertex_array_attr_buffer(&self, vao: VertexArrayId, index: u32) -> BufferId {
        self.res()
            .vertex_arrays
            .get(vao.0)
            .and_then(|v| v.attribs.get(index as usize).copied().flatten())
            .map_or(BufferId::NULL, |(b, _)| b)
    }

    /// Where attribute `index` of `vao` sits in its buffer: `[offset,
    /// stride, divisor]` in bytes (divisor 0: per vertex), or empty if none.
    #[wasm_bindgen(js_name = vertexArrayAttrLayout)]
    pub fn vertex_array_attr_layout(&self, vao: VertexArrayId, index: u32) -> Vec<i32> {
        self.res()
            .vertex_arrays
            .get(vao.0)
            .and_then(|v| v.attribs.get(index as usize).copied().flatten())
            .map_or_else(Vec::new, |(_, l)| l.to_vec())
    }

    /// The index buffer of `vao` (NULL if none).
    #[wasm_bindgen(js_name = vertexArrayIndexBuffer)]
    pub fn vertex_array_index_buffer(&self, vao: VertexArrayId) -> BufferId {
        self.res()
            .vertex_arrays
            .get(vao.0)
            .and_then(|v| v.index_buffer)
            .unwrap_or(BufferId::NULL)
    }

    #[wasm_bindgen(js_name = vertexArrayRemoveAttr)]
    pub fn vertex_array_remove_attr(&self, vao: VertexArrayId, index: u32) {
        if !attr_index_in_range(index) {
            return;
        }
        let gl = self.gl();
        let res = &mut *self.res();
        let Some(v) = res.vertex_arrays.get_mut(vao.0) else {
            return self.stale("vertex array", "vertex_array_remove_attr", vao);
        };
        self.gl_state().forget_vao();
        gl.bind_vertex_array(Some(&v.raw));
        gl.disable_vertex_attrib_array(index);
        gl.bind_vertex_array(None);
        if let Some((prev, _)) = v.attribs.get_mut(index as usize).and_then(Option::take) {
            release_buffer(res, gl, prev);
        }
    }

    /// Uses `buffer` (created with `create_index_buffer`) for indexed draws.
    #[wasm_bindgen(js_name = vertexArraySetIndexBuffer)]
    pub fn vertex_array_set_index_buffer(&self, vao: VertexArrayId, buffer: BufferId) {
        let gl = self.gl();
        let res = &mut *self.res();
        let Some(v) = res.vertex_arrays.get(vao.0) else {
            return self.stale("vertex array", "vertex_array_set_index_buffer", vao);
        };
        let Some(b) = res.buffers.get(buffer.0) else {
            return self.stale("buffer", "vertex_array_set_index_buffer", buffer);
        };
        if b.kind != BufferKind::Index {
            console::error(
                "[luxel] vertex_array_set_index_buffer: buffer wasn't created with \
                 create_index_buffer",
            );
            return;
        }
        self.gl_state().forget_vao();
        gl.bind_vertex_array(Some(&v.raw));
        gl.bind_buffer(WebGl2RenderingContext::ELEMENT_ARRAY_BUFFER, Some(&b.raw));
        gl.bind_vertex_array(None);

        res.buffers.retain(buffer.0);
        let v = res.vertex_arrays.get_mut(vao.0).expect("checked above");
        if let Some(prev) = v.index_buffer.replace(buffer) {
            release_buffer(res, gl, prev);
        }
    }

    #[wasm_bindgen(js_name = vertexArrayRemoveIndexBuffer)]
    pub fn vertex_array_remove_index_buffer(&self, vao: VertexArrayId) {
        let gl = self.gl();
        let res = &mut *self.res();
        let Some(v) = res.vertex_arrays.get_mut(vao.0) else {
            return self.stale("vertex array", "vertex_array_remove_index_buffer", vao);
        };
        self.gl_state().forget_vao();
        gl.bind_vertex_array(Some(&v.raw));
        gl.bind_buffer(WebGl2RenderingContext::ELEMENT_ARRAY_BUFFER, None);
        gl.bind_vertex_array(None);
        if let Some(prev) = v.index_buffer.take() {
            release_buffer(res, gl, prev);
        }
    }

    /// Deletes the vertex array, stops using its buffers, and destroys the
    /// buffers it owns (the ones `VaoBuilder` created for it).
    #[wasm_bindgen(js_name = destroyVertexArray)]
    pub fn destroy_vertex_array(&self, vao: VertexArrayId) {
        let gl = self.gl();
        let res = &mut *self.res();
        let obj = match res.vertex_arrays.destroy(vao.0) {
            Destroyed::Now(obj) => obj,
            Destroyed::Deferred => return,
            Destroyed::Invalid => return self.stale("vertex array", "destroy_vertex_array", vao),
        };
        gl.delete_vertex_array(Some(&obj.raw));
        for id in obj
            .attribs
            .into_iter()
            .flatten()
            .map(|(b, _)| b)
            .chain(obj.index_buffer)
        {
            release_buffer(res, gl, id);
        }
        for id in obj.owned {
            if let Destroyed::Now(b) = res.buffers.destroy(id.0) {
                b.delete(gl);
            }
        }
    }
}
