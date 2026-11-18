// The ground shadow, step 2: one direction of a separable Gaussian blur of
// the darkness (red). Ported from three.js' HorizontalBlurShader /
// VerticalBlurShader (MIT; after cake23.de): 9 taps, σ 2.7, `u_step` apart.

precision highp float;

in vec2 v_ndc;
uniform sampler2D u_src;
uniform vec2 u_step;
out vec4 frag_color;

void main() {
    vec2 uv = v_ndc * 0.5 + 0.5;
    float s = texture(u_src, uv - 4.0 * u_step).r * 0.051;
    s += texture(u_src, uv - 3.0 * u_step).r * 0.0918;
    s += texture(u_src, uv - 2.0 * u_step).r * 0.12245;
    s += texture(u_src, uv - 1.0 * u_step).r * 0.1531;
    s += texture(u_src, uv).r * 0.1633;
    s += texture(u_src, uv + 1.0 * u_step).r * 0.1531;
    s += texture(u_src, uv + 2.0 * u_step).r * 0.12245;
    s += texture(u_src, uv + 3.0 * u_step).r * 0.0918;
    s += texture(u_src, uv + 4.0 * u_step).r * 0.051;
    frag_color = vec4(s, 0.0, 0.0, 1.0);
}
