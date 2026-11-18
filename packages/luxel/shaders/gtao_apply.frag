// The occlusion over the opaque scene: its visibility (red: AO, green:
// the contact shadow), each mixed toward 1 by its intensity, multiplied
// into the target's color by the blend (zero × source + destination ×
// source color).

precision highp float;

in vec2 v_ndc;
uniform sampler2D u_ao;
// The key light's share of each pixel's radiance (r): the part a contact
// shadow (of that light) takes away.
uniform sampler2D u_key;
// x the AO's intensity, y the contact shadow's (0: none, 1: all of it),
// z 1 with u_key (else the contact shadow dims the whole pixel).
uniform vec4 u_apply;
out vec4 frag_color;

void main() {
    vec2 uv = v_ndc * 0.5 + 0.5;
    vec2 v = textureLod(u_ao, uv, 0.0).rg;
    float share = u_apply.z > 0.5 ? textureLod(u_key, uv, 0.0).r : 1.0;
    float k = mix(1.0, v.r, u_apply.x) * mix(1.0, v.g, u_apply.y * share);
    frag_color = vec4(vec3(k), 1.0);
}
