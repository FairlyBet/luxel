// Punctual lights (the Lights block) and the environment's strength (the
// Lighting block), both in slot 0. Light type: 0 directional, 1 point, 2
// spot (KHR_lights_punctual).

#define MAX_LIGHTS 8
#define MAX_SHADOW_VIEWS 16
struct Light {
    vec4 position;  // xyz world position (point, spot)
    vec4 direction; // xyz world direction the light points (directional, spot)
    vec4 color;     // rgb color, w intensity (lux for directional, candela else)
    vec4 params;    // x range (0 = infinite), y spot scale, z spot offset, w type
};
// The punctual lights and their shadows (the lighting system's block).
layout(std140) uniform Lights {
    Light u_lights[MAX_LIGHTS];
    vec4 u_light_count; // x lights in u_lights, y 1: cascades tinted,
                        // z how far cascades reach into the one before,
                        // w the key light's index (−1: none): the one
                        // contact shadows are of
    // Light i's shadow: x its first shadow map (view; < 0: none), y how
    // many (a directional light's cascades, a point light's 6 faces), z its
    // kind: 0 orthographic, 1 a spot's perspective, 2 a point's faces.
    vec4 u_light_shadows[MAX_LIGHTS];
    // Its cascades' far bounds: depth along the view (the last: all).
    vec4 u_shadow_splits[MAX_LIGHTS];
    // Map i: x its normal bias (world units; per unit of depth for a
    // perspective one), y its layer of u_shadow_map; the part of the layer
    // it fills (x0, y0, x1, y1); world → that part's coordinates and depth.
    vec4 u_shadow_views[MAX_SHADOW_VIEWS];
    vec4 u_shadow_tiles[MAX_SHADOW_VIEWS];
    mat4 u_shadow_matrices[MAX_SHADOW_VIEWS];
};
// What the key light (u_light_count.w) adds to the pixel, summed by the
// shading models' light loops: the share contact shadows dim.
vec3 g_key_light = vec3(0.0);

// The key light's share of `color` (radiance), for the second output: what
// the contact shadow, applied after the opaque pass, takes away.
float key_light_share(vec3 color) {
    return clamp(luminance(g_key_light) / max(luminance(color), 1e-6), 0.0, 1.0);
}

// The environment and the output (the renderer's block).
layout(std140) uniform Lighting {
    vec4 u_environment; // rgb intensity of the environment light, w the
                        // material maps' LOD bias (TAA)
    vec4 u_lighting;    // x unused, y exposure, z environment rotation,
                        // w last mip of the specular cube
    vec4 u_output;      // x 1: scene_output shows (exposure, tone map, sRGB),
                        //   0: it leaves linear radiance (HDR target);
                        // y tone mapper (tone_map modes), z 0 without a
                        // scene copy for transmission, else 1 + its least
                        // mip to read (scene_color_available /
                        // scene_color_at), w its last mip
    vec4 u_shading;     // x specular antialiasing variance, y its threshold,
                        // z depth-peeling mode, w 1 under reverse-Z
    vec4 u_fill_a;      // the background fill (background_fill: w its
    vec4 u_fill_b;      // kind, 0 none), in display values
};


// Direction from a surface point toward light `light` (unnormalized for
// point and spot lights: the distance is in it).
vec3 light_vector(Light light, vec3 position) {
    return light.params.w < 0.5 ? -light.direction.xyz : light.position.xyz - position;
}

// Distance and cone falloff (KHR_lights_punctual): the range window
// clamp(1 - (d / range)^4, 0, 1) / d², the spot's smooth cone.
float light_attenuation(Light light, vec3 to_light) {
    float type = light.params.w;
    if (type < 0.5) return 1.0; // directional
    float d2 = max(dot(to_light, to_light), 1e-8);
    float att = 1.0 / d2;
    float range = light.params.x;
    if (range > 0.0) {
        float r = sqrt(d2) / range;
        att *= clamp(1.0 - r * r * r * r, 0.0, 1.0);
    }
    if (type > 1.5) { // spot
        float cd = dot(normalize(light.direction.xyz), -normalize(to_light));
        float s = clamp(cd * light.params.y + light.params.z, 0.0, 1.0);
        att *= s * s;
    }
    return att;
}

// Radiance arriving from `light` at `position`, along `l` (normalized).
vec3 light_radiance(Light light, vec3 position, out vec3 l) {
    vec3 to_light = light_vector(light, position);
    l = normalize(to_light);
    return light.color.rgb * light.color.w * light_attenuation(light, to_light);
}

