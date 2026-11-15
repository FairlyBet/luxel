// The occlusion over the opaque scene: its visibility (red: AO, green:
// the contact shadow), each mixed toward 1 by its intensity, multiplied
// into the target's color by the blend (zero × source + destination ×
// source color).

precision highp float;

in vec2 v_ndc;
uniform sampler2D u_ao;
// x the AO's intensity, y the contact shadow's (0: none, 1: all of it).
uniform vec4 u_apply;
out vec4 frag_color;

void main() {
    vec2 v = textureLod(u_ao, v_ndc * 0.5 + 0.5, 0.0).rg;
    float k = mix(1.0, v.r, u_apply.x) * mix(1.0, v.g, u_apply.y);
    frag_color = vec4(vec3(k), 1.0);
}
