// Ground-truth ambient occlusion (Jimenez et al. 2016, "Practical Real-Time
// Strategies for Accurate Indirect Occlusion") over the opaque scene's
// depth, at the AO target's resolution. Ported from three.js' GTAOShader
// (MIT): normals from the depth (its computeNormalFromDepth), horizons
// searched along DIRECTIONS slices of STEPS samples each side, the
// cosine-weighted visible arc integrated per slice. Its noise texture is
// replaced by interleaved gradient noise (Jimenez 2014) moving each frame,
// which TAA averages. Writes the visibility (1: open) to red.
//
// Green: the contact shadow of one light (Filament's
// screenSpaceContactShadow, Apache-2.0): a short ray from the point toward
// the light, dithered by the same noise, occluded where the scene's surface
// lies in front of it by less than a thickness; faded out at the screen's
// edges (no depth beyond them). Marched in view space, so any depth
// convention reads alike. 1 where off or lit.

precision highp float;

#define PI 3.141592653589793
#define DIRECTIONS 3
#define STEPS 6

in vec2 v_ndc;
uniform highp sampler2D u_depth;
uniform mat4 u_proj;
uniform mat4 u_inv_proj;
// x radius, y thickness (view-space units), z distance exponent, w distance
// falloff.
uniform vec4 u_gtao;
// x the result's power, y 1 under reverse-Z, z frame index (noise), w 1
// with ambient occlusion.
uniform vec4 u_gtao_frame;
// The contact shadow: xyz the light's direction toward it (view space), w
// the ray's length (0: off).
uniform vec4 u_contact;
// x its thickness, y steps.
uniform vec4 u_contact_steps;
out vec4 frag_color;

vec3 view_position(vec2 uv, float depth) {
    vec4 clip = u_gtao_frame.y > 0.5
        ? vec4(uv * 2.0 - 1.0, depth, 1.0)
        : vec4(vec3(uv, depth) * 2.0 - 1.0, 1.0);
    vec4 v = u_inv_proj * clip;
    return v.xyz / v.w;
}

float depth_at(vec2 uv) {
    return textureLod(u_depth, uv, 0.0).r;
}

float fetch_depth(ivec2 p) {
    return texelFetch(u_depth, p, 0).r;
}

// The view-space normal from the depth around `uv`: on each axis the side
// whose depth continues the center's line (no edge across a silhouette).
vec3 normal_from_depth(vec2 uv) {
    vec2 size = vec2(textureSize(u_depth, 0));
    ivec2 p = ivec2(uv * size);
    float c0 = fetch_depth(p);
    float l2 = fetch_depth(p - ivec2(2, 0));
    float l1 = fetch_depth(p - ivec2(1, 0));
    float r1 = fetch_depth(p + ivec2(1, 0));
    float r2 = fetch_depth(p + ivec2(2, 0));
    float b2 = fetch_depth(p - ivec2(0, 2));
    float b1 = fetch_depth(p - ivec2(0, 1));
    float t1 = fetch_depth(p + ivec2(0, 1));
    float t2 = fetch_depth(p + ivec2(0, 2));
    float dl = abs((2.0 * l1 - l2) - c0);
    float dr = abs((2.0 * r1 - r2) - c0);
    float db = abs((2.0 * b1 - b2) - c0);
    float dt = abs((2.0 * t1 - t2) - c0);
    vec3 ce = view_position(uv, c0);
    vec3 dpdx = (dl < dr)
        ? ce - view_position(uv - vec2(1.0 / size.x, 0.0), l1)
        : -ce + view_position(uv + vec2(1.0 / size.x, 0.0), r1);
    vec3 dpdy = (db < dt)
        ? ce - view_position(uv - vec2(0.0, 1.0 / size.y), b1)
        : -ce + view_position(uv + vec2(0.0, 1.0 / size.y), t1);
    return normalize(cross(dpdx, dpdy));
}

vec3 scene_uv_and_depth(vec3 view_pos) {
    vec4 clip = u_proj * vec4(view_pos, 1.0);
    vec2 uv = clip.xy / clip.w * 0.5 + 0.5;
    return vec3(uv, depth_at(uv));
}

// Interleaved gradient noise (Jimenez 2014), shifted per frame.
float ign(vec2 pixel) {
    pixel += 5.588238 * u_gtao_frame.z;
    return fract(52.9829189 * fract(dot(pixel, vec2(0.06711056, 0.00583715))));
}

// 1 lit, 0 in a contact shadow of the light toward `l` (view space).
float contact_shadow(vec3 view_pos, vec3 n, vec3 l) {
    if (dot(n, l) <= 0.0) return 1.0; // facing away: the light shades it
    int steps = int(u_contact_steps.y);
    float dt = 1.0 / float(steps);
    float t = dt * (ign(gl_FragCoord.xy + vec2(7.0, 3.0)) - 0.5) + dt;
    // Starts a hair off the surface, against self-shadowing.
    vec3 start = view_pos + n * (u_contact.w * 0.02);
    vec3 ray = l * u_contact.w;
    vec2 last_uv = vec2(0.5);
    for (int i = 0; i < 64; ++i) {
        if (i >= steps) break;
        vec3 p = start + ray * t;
        vec3 s = scene_uv_and_depth(p);
        last_uv = s.xy;
        if (any(lessThan(s.xy, vec2(0.0))) || any(greaterThan(s.xy, vec2(1.0)))) break;
        float dz = view_position(s.xy, s.z).z - p.z;
        if (dz > 0.0 && dz < u_contact_steps.x) {
            vec2 fade = max(12.0 * abs(last_uv - 0.5) - 5.0, 0.0);
            return 1.0 - clamp(1.0 - dot(fade, fade), 0.0, 1.0);
        }
        t += dt;
    }
    return 1.0;
}

void main() {
    vec2 uv = v_ndc * 0.5 + 0.5;
    float depth = depth_at(uv);
    bool far = u_gtao_frame.y > 0.5 ? depth <= 0.0 : depth >= 1.0;
    if (far) {
        frag_color = vec4(1.0);
        return;
    }
    vec3 view_pos = view_position(uv, depth);
    vec3 view_normal = normal_from_depth(uv);
    float contact = u_contact.w > 0.0 ? contact_shadow(view_pos, view_normal, u_contact.xyz) : 1.0;
    if (u_gtao_frame.w < 0.5) {
        frag_color = vec4(1.0, contact, 0.0, 1.0);
        return;
    }
    float radius = u_gtao.x;
    float thickness = u_gtao.y;

    float n0 = ign(gl_FragCoord.xy);
    float n1 = ign(gl_FragCoord.xy + vec2(17.0, 31.0));
    vec3 tangent = vec3(cos(n0 * 2.0 * PI), sin(n0 * 2.0 * PI), 0.0);
    vec3 bitangent = vec3(-tangent.y, tangent.x, 0.0);
    mat3 kernel = mat3(tangent, bitangent, vec3(0.0, 0.0, 1.0));
    // Perspective: toward the eye; orthographic (w column = 0 on xy).
    vec3 view_dir = u_proj[3][3] < 0.5 ? normalize(-view_pos) : vec3(0.0, 0.0, 1.0);

    float ao = 0.0;
    for (int i = 0; i < DIRECTIONS; ++i) {
        float angle = float(i) / float(DIRECTIONS) * PI;
        vec4 dir = vec4(cos(angle), sin(angle), 0.0, 0.5 + 0.5 * n1);
        dir.xyz = normalize(kernel * dir.xyz);
        vec3 slice_bitangent = normalize(cross(dir.xyz, view_dir));
        vec3 slice_tangent = cross(slice_bitangent, view_dir);
        vec3 normal_in_slice = normalize(view_normal - slice_bitangent * dot(view_normal, slice_bitangent));
        vec3 to_normal = cross(normal_in_slice, slice_bitangent);
        vec2 cos_h = vec2(dot(view_dir, to_normal), dot(view_dir, -to_normal));
        for (int j = 0; j < STEPS; ++j) {
            vec3 offset = dir.xyz * radius * dir.w * pow(float(j + 1) / float(STEPS), u_gtao.z);
            float falloff = mix(1.0, 2.0 / float(j + 2), u_gtao.w);
            vec3 s = scene_uv_and_depth(view_pos + offset);
            vec3 delta = view_position(s.xy, s.z) - view_pos;
            if (abs(delta.z) < thickness) {
                cos_h.x += max(0.0, (dot(view_dir, normalize(delta)) - cos_h.x) * falloff);
            }
            s = scene_uv_and_depth(view_pos - offset);
            delta = view_position(s.xy, s.z) - view_pos;
            if (abs(delta.z) < thickness) {
                cos_h.y += max(0.0, (dot(view_dir, normalize(delta)) - cos_h.y) * falloff);
            }
        }
        cos_h = clamp(cos_h, -1.0, 1.0);
        vec2 sin_h = sqrt(1.0 - cos_h * cos_h);
        float nx = dot(normal_in_slice, slice_tangent);
        float ny = dot(normal_in_slice, view_dir);
        float nxb = 0.5 * (acos(cos_h.y) - acos(cos_h.x) + sin_h.x * cos_h.x - sin_h.y * cos_h.y);
        float nyb = 0.5 * (2.0 - cos_h.x * cos_h.x - cos_h.y * cos_h.y);
        ao += nx * nxb + ny * nyb;
    }
    ao = clamp(ao / float(DIRECTIONS), 0.0, 1.0);
    frag_color = vec4(pow(ao, u_gtao_frame.x), contact, 0.0, 1.0);
}
