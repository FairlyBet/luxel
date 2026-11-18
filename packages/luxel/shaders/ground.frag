// The ground shadow's plane (on the scene prelude): black, as opaque as
// the blurred darkness under it, blended over what is behind.

in vec3 v_world;
uniform sampler2D u_ground;
// World → the darkness texture's coordinates (the view from the ground).
uniform mat4 u_ground_map;
uniform float u_ground_opacity;
layout(location = 0) out vec4 frag_color;
// No key light here (contact shadows' second attachment).
layout(location = 1) out vec4 frag_key;

void main() {
    vec4 p = u_ground_map * vec4(v_world, 1.0);
    float a = texture(u_ground, p.xy).r * u_ground_opacity;
    frag_color = vec4(scene_output(vec3(0.0)), a);
    frag_key = vec4(0.0);
}
