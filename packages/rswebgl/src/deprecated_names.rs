//! Old names kept for one release after the core's predicates and uniform
//! buffer calls moved to the `<resource><Verb>` style the rest of the API
//! uses (`bufferIsValid`, `programIsReady`, `textureSetMinFilter`). Each one
//! forwards to its new name; Rust callers get a deprecation warning, JS
//! callers the `@deprecated` tag in the generated typings.

#![allow(deprecated)]

use wasm_bindgen::prelude::*;

use crate::context::Context;
use crate::ids::{FramebufferId, QueryId, UniformBufferId};

#[wasm_bindgen]
impl Context {
    /// @deprecated Use `framebufferIsComplete`.
    #[deprecated(note = "use `framebuffer_is_complete`")]
    #[wasm_bindgen(js_name = isFramebufferComplete)]
    pub fn is_framebuffer_complete(&self, fb: FramebufferId) -> bool {
        self.framebuffer_is_complete(fb)
    }

    /// @deprecated Use `queryIsPending`.
    #[deprecated(note = "use `query_is_pending`")]
    #[wasm_bindgen(js_name = queryPending)]
    pub fn query_pending(&self, q: QueryId) -> bool {
        self.query_is_pending(q)
    }

    /// @deprecated Use `uniformBufferUpload`.
    #[deprecated(note = "use `uniform_buffer_upload`")]
    #[wasm_bindgen(js_name = uboUpload)]
    pub fn ubo_upload(&self, ubo: UniformBufferId) {
        self.uniform_buffer_upload(ubo)
    }

    /// @deprecated Use `uniformBufferByteSize`.
    #[deprecated(note = "use `uniform_buffer_byte_size`")]
    #[wasm_bindgen(js_name = uboByteSize)]
    pub fn ubo_byte_size(&self, ubo: UniformBufferId) -> u32 {
        self.uniform_buffer_byte_size(ubo)
    }

    /// @deprecated Use `uniformBufferData`.
    #[deprecated(note = "use `uniform_buffer_data`")]
    #[wasm_bindgen(js_name = uboData)]
    pub fn ubo_data(&self, ubo: UniformBufferId) -> Vec<u8> {
        self.uniform_buffer_data(ubo)
    }

    /// @deprecated Use `uniformBufferSetF32`.
    #[deprecated(note = "use `uniform_buffer_set_f32`")]
    #[wasm_bindgen(js_name = uboSetF32)]
    pub fn ubo_set_f32(&self, ubo: UniformBufferId, path: &str, x: f32) {
        self.uniform_buffer_set_f32(ubo, path, x)
    }

    /// @deprecated Use `uniformBufferSetVec2`.
    #[deprecated(note = "use `uniform_buffer_set_vec2`")]
    #[wasm_bindgen(js_name = uboSetVec2)]
    pub fn ubo_set_vec2(&self, ubo: UniformBufferId, path: &str, x: f32, y: f32) {
        self.uniform_buffer_set_vec2(ubo, path, x, y)
    }

    /// @deprecated Use `uniformBufferSetVec3`.
    #[deprecated(note = "use `uniform_buffer_set_vec3`")]
    #[wasm_bindgen(js_name = uboSetVec3)]
    pub fn ubo_set_vec3(&self, ubo: UniformBufferId, path: &str, x: f32, y: f32, z: f32) {
        self.uniform_buffer_set_vec3(ubo, path, x, y, z)
    }

    /// @deprecated Use `uniformBufferSetVec4`.
    #[deprecated(note = "use `uniform_buffer_set_vec4`")]
    #[wasm_bindgen(js_name = uboSetVec4)]
    pub fn ubo_set_vec4(&self, ubo: UniformBufferId, path: &str, x: f32, y: f32, z: f32, w: f32) {
        self.uniform_buffer_set_vec4(ubo, path, x, y, z, w)
    }

    /// @deprecated Use `uniformBufferSetI32`.
    #[deprecated(note = "use `uniform_buffer_set_i32`")]
    #[wasm_bindgen(js_name = uboSetI32)]
    pub fn ubo_set_i32(&self, ubo: UniformBufferId, path: &str, x: i32) {
        self.uniform_buffer_set_i32(ubo, path, x)
    }

    /// @deprecated Use `uniformBufferSetIvec2`.
    #[deprecated(note = "use `uniform_buffer_set_ivec2`")]
    #[wasm_bindgen(js_name = uboSetIvec2)]
    pub fn ubo_set_ivec2(&self, ubo: UniformBufferId, path: &str, x: i32, y: i32) {
        self.uniform_buffer_set_ivec2(ubo, path, x, y)
    }

    /// @deprecated Use `uniformBufferSetIvec3`.
    #[deprecated(note = "use `uniform_buffer_set_ivec3`")]
    #[wasm_bindgen(js_name = uboSetIvec3)]
    pub fn ubo_set_ivec3(&self, ubo: UniformBufferId, path: &str, x: i32, y: i32, z: i32) {
        self.uniform_buffer_set_ivec3(ubo, path, x, y, z)
    }

    /// @deprecated Use `uniformBufferSetIvec4`.
    #[deprecated(note = "use `uniform_buffer_set_ivec4`")]
    #[wasm_bindgen(js_name = uboSetIvec4)]
    pub fn ubo_set_ivec4(&self, ubo: UniformBufferId, path: &str, x: i32, y: i32, z: i32, w: i32) {
        self.uniform_buffer_set_ivec4(ubo, path, x, y, z, w)
    }

    /// @deprecated Use `uniformBufferSetU32`.
    #[deprecated(note = "use `uniform_buffer_set_u32`")]
    #[wasm_bindgen(js_name = uboSetU32)]
    pub fn ubo_set_u32(&self, ubo: UniformBufferId, path: &str, x: u32) {
        self.uniform_buffer_set_u32(ubo, path, x)
    }

    /// @deprecated Use `uniformBufferSetUvec2`.
    #[deprecated(note = "use `uniform_buffer_set_uvec2`")]
    #[wasm_bindgen(js_name = uboSetUvec2)]
    pub fn ubo_set_uvec2(&self, ubo: UniformBufferId, path: &str, x: u32, y: u32) {
        self.uniform_buffer_set_uvec2(ubo, path, x, y)
    }

    /// @deprecated Use `uniformBufferSetUvec3`.
    #[deprecated(note = "use `uniform_buffer_set_uvec3`")]
    #[wasm_bindgen(js_name = uboSetUvec3)]
    pub fn ubo_set_uvec3(&self, ubo: UniformBufferId, path: &str, x: u32, y: u32, z: u32) {
        self.uniform_buffer_set_uvec3(ubo, path, x, y, z)
    }

    /// @deprecated Use `uniformBufferSetUvec4`.
    #[deprecated(note = "use `uniform_buffer_set_uvec4`")]
    #[wasm_bindgen(js_name = uboSetUvec4)]
    pub fn ubo_set_uvec4(&self, ubo: UniformBufferId, path: &str, x: u32, y: u32, z: u32, w: u32) {
        self.uniform_buffer_set_uvec4(ubo, path, x, y, z, w)
    }

    /// @deprecated Use `uniformBufferSetBool`.
    #[deprecated(note = "use `uniform_buffer_set_bool`")]
    #[wasm_bindgen(js_name = uboSetBool)]
    pub fn ubo_set_bool(&self, ubo: UniformBufferId, path: &str, value: bool) {
        self.uniform_buffer_set_bool(ubo, path, value)
    }

    /// @deprecated Use `uniformBufferSetMat`.
    #[deprecated(note = "use `uniform_buffer_set_mat`")]
    #[wasm_bindgen(js_name = uboSetMat)]
    pub fn ubo_set_mat(&self, ubo: UniformBufferId, path: &str, data: &[f32]) {
        self.uniform_buffer_set_mat(ubo, path, data)
    }
}
