// Shadows of the punctual lights (lights.glsl): one depth layer per
// shadow-casting light, compared in hardware (LEQUAL, GEQUAL under
// reverse-Z: 1 lit, 0 shadowed). Needs lights.glsl.

uniform highp sampler2DArrayShadow u_shadow_map;

// How much of light `index` reaches `position` past the shadow casters (1
// lit, 0 shadowed), filtered 3×3. `ng`: the geometric normal, to offset
// along. Ported from Filament (Apache-2.0): the normal bias of
// computeLightSpacePosition (surface_shadowing.glsl) and the 4-tap filter
// of ShadowSample_PCF_Low (surface_shadowing.fs: Castaño 2013, "Shadow
// Mapping Summary Part 1"), with a whole layer per light.
float light_shadow(int index, Light light, vec3 position, vec3 ng) {
    vec4 shadow = u_light_shadows[index];
    float layer = shadow.x;
    if (layer < 0.0) return 1.0;
    // Away along the normal, more as the light grazes the surface.
    vec3 l = normalize(light_vector(light, position));
    float cos_t = clamp(dot(ng, l), 0.0, 1.0);
    float sin_t = sqrt(1.0 - cos_t * cos_t);
    float bias = shadow.y;
    if (shadow.z > 0.5) bias *= max(dot(position - light.position.xyz, light.direction.xyz), 0.0);
    vec4 sp = u_shadow_matrices[int(layer)] * vec4(position + ng * (sin_t * bias), 1.0);
    vec3 p = sp.xyz / sp.w;
    // Outside what the map covers: lit.
    if (any(lessThan(p, vec3(0.0))) || any(greaterThan(p, vec3(1.0)))) return 1.0;

    vec2 size = vec2(textureSize(u_shadow_map, 0).xy);
    vec2 texel = 1.0 / size;
    vec2 uv = p.xy * size + 0.5;
    vec2 base = (floor(uv) - 0.5) * texel;
    vec2 st = fract(uv);
    vec2 uw = vec2(3.0 - 2.0 * st.x, 1.0 + 2.0 * st.x);
    vec2 vw = vec2(3.0 - 2.0 * st.y, 1.0 + 2.0 * st.y);
    vec2 u = vec2((2.0 - st.x) / uw.x - 1.0, st.x / uw.y + 1.0) * texel.x;
    vec2 v = vec2((2.0 - st.y) / vw.x - 1.0, st.y / vw.y + 1.0) * texel.y;
    float sum = uw.x * vw.x * texture(u_shadow_map, vec4(base + vec2(u.x, v.x), layer, p.z));
    sum += uw.y * vw.x * texture(u_shadow_map, vec4(base + vec2(u.y, v.x), layer, p.z));
    sum += uw.x * vw.y * texture(u_shadow_map, vec4(base + vec2(u.x, v.y), layer, p.z));
    sum += uw.y * vw.y * texture(u_shadow_map, vec4(base + vec2(u.y, v.y), layer, p.z));
    return sum * 0.0625;
}
