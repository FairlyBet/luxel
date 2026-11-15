// The occlusion's noise smoothed (both channels: AO, contact shadow): a
// 4×4 blur of its texels weighted by how close their depth is to this
// pixel's (so it doesn't bleed across silhouettes), as the spatial half of XeGTAO's denoise. Reads the AO at
// its own resolution, the depth at full.

precision highp float;

in vec2 v_ndc;
uniform sampler2D u_ao;
uniform highp sampler2D u_depth;
// x 1 under reverse-Z.
uniform vec4 u_denoise;
out vec4 frag_color;

float linear_depth(float d) {
    // Only differences matter: depth itself is close enough to linear
    // within a few texels, but nearer depth is finer — scale by it.
    return u_denoise.x > 0.5 ? 1.0 / max(d, 1e-6) : 1.0 / max(1.0 - d, 1e-6);
}

void main() {
    vec2 uv = v_ndc * 0.5 + 0.5;
    vec2 texel = 1.0 / vec2(textureSize(u_ao, 0));
    float center = linear_depth(textureLod(u_depth, uv, 0.0).r);
    vec2 sum = vec2(0.0);
    float weights = 0.0;
    for (int y = -2; y < 2; ++y) {
        for (int x = -2; x < 2; ++x) {
            vec2 o = (vec2(x, y) + 0.5) * texel;
            float d = linear_depth(textureLod(u_depth, uv + o, 0.0).r);
            float w = 1.0 / (1e-3 + abs(d - center) / max(center, 1e-6) * 50.0);
            sum += textureLod(u_ao, uv + o, 0.0).rg * w;
            weights += w;
        }
    }
    frag_color = vec4(sum / max(weights, 1e-6), 0.0, 1.0);
}
