// The occlusion over the opaque scene: its visibility (red), mixed toward
// 1 by the intensity, multiplied into the target's color by the blend
// (zero × source + destination × source color).

precision highp float;

in vec2 v_ndc;
uniform sampler2D u_ao;
// x intensity (0: none, 1: all of it).
uniform vec4 u_apply;
out vec4 frag_color;

void main() {
    float v = textureLod(u_ao, v_ndc * 0.5 + 0.5, 0.0).r;
    frag_color = vec4(vec3(mix(1.0, v, u_apply.x)), 1.0);
}
