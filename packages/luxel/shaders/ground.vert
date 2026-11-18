// The ground shadow's plane (on the scene prelude): a unit square in its
// object's XZ, placed under the casters by its instance.

layout(location = 0) in vec3 a_position;
out vec3 v_world;

void main() {
    vec4 world = u_model * vec4(a_position, 1.0);
    v_world = world.xyz;
    gl_Position = u_view_proj * world;
}
