// The environment as the background: the specular cube looked up along each
// pixel's view ray, at a mip for blur, through the same output transform as
// the lit scene. Drawn first, without depth, by SceneRenderer. Comes after
// the scene prelude (`scene.frag.glsl`).

in vec2 v_ndc;
layout(location = 0) out vec4 frag_color;
// No key light here (contact shadows' second attachment).
layout(location = 1) out vec4 frag_key;

uniform float u_background_blur; // 0 sharp … 1 the roughest mip

void main() {
    // The background fill, drawing straight to the canvas (to an HDR
    // target the display pass lays it under the scene instead).
    if (u_fill_a.w > 0.5) {
        frag_color = vec4(background_fill(v_ndc * 0.5 + 0.5, u_fill_a, u_fill_b), 1.0);
        frag_key = vec4(0.0);
        return;
    }
    // View-space ray through this pixel, then to world space (the view's
    // rotation is orthonormal: its inverse is its transpose).
    vec3 view_ray = vec3(v_ndc.x / u_proj[0][0], v_ndc.y / u_proj[1][1], -1.0);
    vec3 dir = normalize(transpose(mat3(u_view)) * view_ray);
    frag_color = vec4(scene_output(scene_reflection(dir, u_background_blur)), 1.0);
    frag_key = vec4(0.0);
}
