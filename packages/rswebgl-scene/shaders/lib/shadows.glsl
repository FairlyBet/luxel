// Shadows of the punctual lights (lights.glsl): depth layers compared in
// hardware (LEQUAL, GEQUAL under reverse-Z: 1 lit, 0 shadowed) — one per
// spot light, one per cascade of a directional light. Needs lights.glsl and
// frame.glsl (the view, to pick a cascade).

uniform highp sampler2DArrayShadow u_shadow_map;

// How much of a light reaches `position` past the casters of map `layer`
// (1 lit, 0 shadowed), filtered 3×3; `perspective` for a spot's map.
// `ng`: the geometric normal to offset along, `sin_t` the sine of the
// light's angle to it. Ported from Filament (Apache-2.0): the normal bias
// of computeLightSpacePosition (surface_shadowing.glsl) and the 4-tap
// filter of ShadowSample_PCF_Low (surface_shadowing.fs: Castaño 2013,
// "Shadow Mapping Summary Part 1").
float shadow_layer(int layer, bool perspective, Light light, vec3 position, vec3 ng, float sin_t) {
    // Away along the normal, more as the light grazes the surface.
    float bias = u_shadow_layers[layer].x;
    if (perspective) bias *= max(dot(position - light.position.xyz, light.direction.xyz), 0.0);
    vec4 sp = u_shadow_matrices[layer] * vec4(position + ng * (sin_t * bias), 1.0);
    vec3 p = sp.xyz / sp.w;
    // Outside what the map covers: lit.
    if (any(lessThan(p, vec3(0.0))) || any(greaterThan(p, vec3(1.0)))) return 1.0;

    float z = float(layer);
    vec2 size = vec2(textureSize(u_shadow_map, 0).xy);
    vec2 texel = 1.0 / size;
    vec2 uv = p.xy * size + 0.5;
    vec2 base = (floor(uv) - 0.5) * texel;
    vec2 st = fract(uv);
    vec2 uw = vec2(3.0 - 2.0 * st.x, 1.0 + 2.0 * st.x);
    vec2 vw = vec2(3.0 - 2.0 * st.y, 1.0 + 2.0 * st.y);
    vec2 u = vec2((2.0 - st.x) / uw.x - 1.0, st.x / uw.y + 1.0) * texel.x;
    vec2 v = vec2((2.0 - st.y) / vw.x - 1.0, st.y / vw.y + 1.0) * texel.y;
    float sum = uw.x * vw.x * texture(u_shadow_map, vec4(base + vec2(u.x, v.x), z, p.z));
    sum += uw.y * vw.x * texture(u_shadow_map, vec4(base + vec2(u.y, v.x), z, p.z));
    sum += uw.x * vw.y * texture(u_shadow_map, vec4(base + vec2(u.x, v.y), z, p.z));
    sum += uw.y * vw.y * texture(u_shadow_map, vec4(base + vec2(u.y, v.y), z, p.z));
    return sum * 0.0625;
}

// Light `index`'s shadow at `position` (x: 1 lit, 0 shadowed) and the
// cascade it came from (y; −1 without a shadow). A directional light's
// cascade is picked by the depth along the view; where the next one
// overlaps it, the two blend (as Bevy's sample_directional_cascade).
vec2 light_shadow_cascade(int index, Light light, vec3 position, vec3 ng) {
    vec4 shadow = u_light_shadows[index];
    if (shadow.x < 0.0) return vec2(1.0, -1.0);
    vec3 l = normalize(light_vector(light, position));
    float cos_t = clamp(dot(ng, l), 0.0, 1.0);
    float sin_t = sqrt(1.0 - cos_t * cos_t);
    int first = int(shadow.x);
    int count = int(shadow.y);
    bool perspective = shadow.z > 0.5;
    if (count <= 1) return vec2(shadow_layer(first, perspective, light, position, ng, sin_t), 0.0);

    float depth = -(u_view * vec4(position, 1.0)).z;
    vec4 splits = u_shadow_splits[index];
    int c = int(depth >= splits.x) + int(depth >= splits.y) + int(depth >= splits.z);
    c = min(c, count - 1);
    float s = shadow_layer(first + c, perspective, light, position, ng, sin_t);
    if (c + 1 < count) {
        float far = c == 0 ? splits.x : (c == 1 ? splits.y : splits.z);
        float next_near = far * (1.0 - u_light_count.z);
        if (depth > next_near) {
            float next = shadow_layer(first + c + 1, perspective, light, position, ng, sin_t);
            s = mix(s, next, (depth - next_near) / max(far - next_near, 1e-6));
        }
    }
    return vec2(s, float(c));
}

// How much of light `index` reaches `position` past the shadow casters (1
// lit, 0 shadowed). `ng`: the geometric normal, to offset along.
float light_shadow(int index, Light light, vec3 position, vec3 ng) {
    return light_shadow_cascade(index, light, position, ng).x;
}

// `light_shadow` as a color: tinted by cascade while they are debugged
// (red, green, blue, yellow nearest first), shadowed parts lit a little so
// their tint shows too.
vec3 light_shadow_rgb(int index, Light light, vec3 position, vec3 ng) {
    vec2 s = light_shadow_cascade(index, light, position, ng);
    if (u_light_count.y < 0.5 || s.y < 0.0) return vec3(s.x);
    int c = int(s.y);
    vec3 tint = c == 0 ? vec3(1.0, 0.3, 0.3)
        : c == 1 ? vec3(0.3, 1.0, 0.3)
        : c == 2 ? vec3(0.3, 0.3, 1.0)
        : vec3(1.0, 1.0, 0.3);
    return max(s.x, 0.25) * tint;
}
