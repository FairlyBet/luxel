// The ground shadow, step 1: the casters' depth seen from the ground up
// (u_depth) as darkness — full where a caster touches the ground, none at
// the top of the reach. As three.js' contact-shadows example
// (webgl_shadow_contact.html, MIT): (1 − depth) × darkness.

precision highp float;

in vec2 v_ndc;
uniform highp sampler2D u_depth;
// x darkness, y 1 under reverse-Z (depth 1 at the ground).
uniform vec4 u_ground_params;
out vec4 frag_color;

void main() {
    float d = texture(u_depth, v_ndc * 0.5 + 0.5).r;
    float from_ground = u_ground_params.y > 0.5 ? 1.0 - d : d;
    frag_color = vec4(clamp((1.0 - from_ground) * u_ground_params.x, 0.0, 1.0), 0.0, 0.0, 1.0);
}
