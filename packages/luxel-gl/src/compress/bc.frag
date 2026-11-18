// Block compression on the GPU: BC1 (DXT1), BC3 (DXT5: a BC4 alpha block and
// a BC1 color block) and BC5 (RGTC2: two BC4 blocks), from an RGBA8 / sRGB
// texture's level. Ported from Betsy (Matias N. Goldberg, MIT; as in Godot,
// modules/betsy: bc1.glsl, bc4.glsl), itself stb_dxt's encoder by Fabian
// Giesen and Sean Barrett (MIT / public domain).
//
// Changes from the original: a fragment shader (GLSL ES 3.00) instead of a
// compute shader — each fragment of an RGBA32UI target encodes one 16-byte
// block (BC3, BC5) or two 8-byte ones (BC1), the blocks numbered row by row
// across the level, so a readPixels of the target is the level's block data;
// texels kept as 0..255 floats (ES 3.00 has no packUnorm4x8); the 5/6-bit
// match tables in an RG8UI texture instead of a storage buffer; BC4 done by
// one invocation per block (no shared memory); edge blocks clamp to the
// level; sRGB sources encoded back to their stored values; no dithering;
// two refinement passes, as Godot's default.
//
// Defines: MODE_BC1, MODE_BC3 or MODE_BC5; SRGB when the source is sRGB.

precision highp float;
precision highp int;
precision highp sampler2D;
precision highp usampler2D;

uniform sampler2D u_source;
uniform usampler2D u_tables; // row 0: stb__OMatch5, row 1: stb__OMatch6
uniform int u_level;
uniform int u_blocks_x; // blocks across the level
uniform int u_blocks;   // blocks in the level
uniform int u_width;    // the target's width, in texels

out uvec4 frag_color;

const float FLT_MAX = 3.402823466e+38;
const int REFINEMENTS = 2;

vec3 srgb_encode(vec3 c) {
    c = clamp(c, 0.0, 1.0);
    return mix(c * 12.92, 1.055 * pow(c, vec3(1.0 / 2.4)) - 0.055, step(0.0031308, c));
}

// The block's 16 texels (row by row), 0..255.
vec4 px[16];

void load_block(int block) {
    ivec2 size = textureSize(u_source, u_level);
    ivec2 base = ivec2(block % u_blocks_x, block / u_blocks_x) * 4;
    for (int i = 0; i < 16; ++i) {
        ivec2 p = min(base + ivec2(i & 3, i >> 2), size - 1);
        vec4 c = texelFetch(u_source, p, u_level);
#ifdef SRGB
        c.rgb = srgb_encode(c.rgb);
#endif
        px[i] = floor(c * 255.0 + 0.5);
    }
}

// --- BC1 -----------------------------------------------------------------

vec2 match5(float v) {
    return vec2(texelFetch(u_tables, ivec2(int(v), 0), 0).rg);
}
vec2 match6(float v) {
    return vec2(texelFetch(u_tables, ivec2(int(v), 1), 0).rg);
}

vec3 rgb565to888(float rgb565) {
    vec3 v;
    v.x = floor(rgb565 / 2048.0);
    v.y = floor(mod(rgb565, 2048.0) / 32.0);
    v.z = floor(mod(rgb565, 32.0));
    // stb_dxt's expansion (Betsy keeps it).
    return floor(v * vec3(8.25, 4.0625, 8.25));
}

float rgb888to565(vec3 c) {
    c.rb = floor(c.rb * 31.0 / 255.0 + 0.5);
    c.g = floor(c.g * 63.0 / 255.0 + 0.5);
    return c.r * 2048.0 + c.g * 32.0 + c.b;
}

// Without rounding bias, as stb_dxt by default.
vec3 lerp13(vec3 a, vec3 b) {
    return floor((2.0 * a + b) / 3.0);
}

void eval_colors(out vec3 colors[4], float c0, float c1) {
    colors[0] = rgb565to888(c0);
    colors[1] = rgb565to888(c1);
    colors[2] = lerp13(colors[0], colors[1]);
    colors[3] = lerp13(colors[1], colors[0]);
}

// Endpoints along the principal axis of the block's colors.
void optimize_colors(out float min16, out float max16) {
    vec3 avg = px[0].rgb, lo = px[0].rgb, hi = px[0].rgb;
    for (int i = 1; i < 16; ++i) {
        avg += px[i].rgb;
        lo = min(lo, px[i].rgb);
        hi = max(hi, px[i].rgb);
    }
    avg = floor(avg / 16.0 + 0.5);

    float cov[6];
    for (int i = 0; i < 6; ++i) cov[i] = 0.0;
    for (int i = 0; i < 16; ++i) {
        vec3 d = px[i].rgb - avg;
        cov[0] += d.r * d.r;
        cov[1] += d.r * d.g;
        cov[2] += d.r * d.b;
        cov[3] += d.g * d.g;
        cov[4] += d.g * d.b;
        cov[5] += d.b * d.b;
    }
    for (int i = 0; i < 6; ++i) cov[i] /= 255.0;

    vec3 vf = hi - lo;
    for (int iter = 0; iter < 4; ++iter) {
        float r = vf.r * cov[0] + vf.g * cov[1] + vf.b * cov[2];
        float g = vf.r * cov[1] + vf.g * cov[3] + vf.b * cov[4];
        float b = vf.r * cov[2] + vf.g * cov[4] + vf.b * cov[5];
        vf = vec3(r, g, b);
    }
    float magn = max(abs(vf.r), max(abs(vf.g), abs(vf.b)));
    vec3 v = magn < 4.0 ? vec3(299.0, 587.0, 114.0) : trunc(vf * (512.0 / magn));

    vec3 min_end = px[0].rgb, max_end = px[0].rgb;
    float min_dot = FLT_MAX, max_dot = -FLT_MAX;
    for (int i = 0; i < 16; ++i) {
        float d = dot(px[i].rgb, v);
        if (d < min_dot) {
            min_dot = d;
            min_end = px[i].rgb;
        }
        if (d > max_dot) {
            max_dot = d;
            max_end = px[i].rgb;
        }
    }
    min16 = rgb888to565(min_end);
    max16 = rgb888to565(max_end);
}

uint match_colors(vec3 color[4]) {
    uint mask = 0u;
    vec3 dir = color[0] - color[1];
    float stops[4];
    for (int i = 0; i < 4; ++i) stops[i] = dot(color[i], dir);
    float c0_point = trunc((stops[1] + stops[3]) * 0.5);
    float half_point = trunc((stops[3] + stops[2]) * 0.5);
    float c3_point = trunc((stops[2] + stops[0]) * 0.5);
    for (int i = 15; i >= 0; --i) {
        float d = dot(px[i].rgb, dir);
        mask <<= 2u;
        if (d < half_point) {
            mask |= (d < c0_point) ? 1u : 3u;
        } else {
            mask |= (d < c3_point) ? 2u : 0u;
        }
    }
    return mask;
}

// Least-squares endpoints for the current indices; whether they changed.
bool refine(uint mask, inout float min16, inout float max16) {
    float new_min, new_max;
    float old_min = min16, old_max = max16;
    if ((mask ^ (mask << 2u)) < 4u) {
        // One index for all: the best single-color match of the average.
        vec3 rgb = vec3(8.0);
        for (int i = 0; i < 16; ++i) rgb += px[i].rgb;
        rgb = floor(rgb / 16.0);
        vec2 r = match5(rgb.r), g = match6(rgb.g), b = match5(rgb.b);
        new_max = r.x * 2048.0 + g.x * 32.0 + b.x;
        new_min = r.y * 2048.0 + g.y * 32.0 + b.y;
    } else {
        float w1_tab[4];
        w1_tab[0] = 3.0;
        w1_tab[1] = 0.0;
        w1_tab[2] = 2.0;
        w1_tab[3] = 1.0;
        float prods[4];
        prods[0] = 589824.0;
        prods[1] = 2304.0;
        prods[2] = 262402.0;
        prods[3] = 66562.0;
        float akku = 0.0;
        uint cm = mask;
        vec3 at1 = vec3(0.0), at2 = vec3(0.0);
        for (int i = 0; i < 16; ++i) {
            int s = int(cm & 3u);
            cm >>= 2u;
            akku += prods[s];
            at1 += px[i].rgb * w1_tab[s];
            at2 += px[i].rgb;
        }
        at2 = 3.0 * at2 - at1;
        float xx = floor(akku / 65535.0);
        float yy = floor(mod(akku, 65535.0) / 256.0);
        float xy = mod(akku, 256.0);
        vec2 f_rb_g;
        f_rb_g.x = 3.0 * 31.0 / 255.0 / (xx * yy - xy * xy);
        f_rb_g.y = f_rb_g.x * 63.0 / 31.0;
        vec3 hi = clamp(floor((at1 * yy - at2 * xy) * f_rb_g.xyx + 0.5), vec3(0.0), vec3(31.0, 63.0, 31.0));
        new_max = hi.x * 2048.0 + hi.y * 32.0 + hi.z;
        vec3 lo = clamp(floor((at2 * xx - at1 * xy) * f_rb_g.xyx + 0.5), vec3(0.0), vec3(31.0, 63.0, 31.0));
        new_min = lo.x * 2048.0 + lo.y * 32.0 + lo.z;
    }
    min16 = new_min;
    max16 = new_max;
    return old_min != new_min || old_max != new_max;
}

uvec2 bc1_block() {
    bool all_equal = true;
    for (int i = 1; i < 16; ++i) all_equal = all_equal && px[i].rgb == px[0].rgb;
    float max16, min16;
    uint mask = 0u;
    if (all_equal) {
        vec2 r = match5(px[0].r), g = match6(px[0].g), b = match5(px[0].b);
        mask = 0xAAAAAAAAu;
        max16 = r.x * 2048.0 + g.x * 32.0 + b.x;
        min16 = r.y * 2048.0 + g.y * 32.0 + b.y;
    } else {
        optimize_colors(min16, max16);
        vec3 colors[4];
        if (min16 != max16) {
            eval_colors(colors, max16, min16);
            mask = match_colors(colors);
        }
        bool stop = false;
        for (int i = 0; i < REFINEMENTS && !stop; ++i) {
            uint last = mask;
            if (refine(mask, min16, max16)) {
                if (min16 != max16) {
                    eval_colors(colors, max16, min16);
                    mask = match_colors(colors);
                } else {
                    mask = 0u;
                    stop = true;
                }
            }
            stop = mask == last || stop;
        }
    }
    // Four-color mode: color0 > color1.
    if (max16 < min16) {
        float t = min16;
        min16 = max16;
        max16 = t;
        mask ^= 0x55555555u;
    }
    return uvec2(uint(max16) | (uint(min16) << 16u), mask);
}

// --- BC4 -----------------------------------------------------------------

// One channel (0..3) of the block, 8 interpolated values.
uvec2 bc4_block(int channel) {
    float v[16];
    float lo = 255.0, hi = 0.0;
    for (int i = 0; i < 16; ++i) {
        v[i] = px[i][channel];
        lo = min(lo, v[i]);
        hi = max(hi, v[i]);
    }
    // Index selection: fgiesen, "DXT5 alpha block index determination".
    float dist = hi - lo;
    float dist4 = dist * 4.0;
    float dist2 = dist * 2.0;
    float bias = (dist < 8.0) ? (dist - 1.0) : (trunc(dist * 0.5) + 2.0);
    bias -= lo * 7.0;
    uint mask0 = 0u, mask1 = 0u;
    for (int i = 0; i < 16; ++i) {
        float a = v[i] * 7.0 + bias;
        int ind = 0;
        if (a >= dist4) {
            ind = 4;
            a -= dist4;
        }
        if (a >= dist2) {
            ind += 2;
            a -= dist2;
        }
        if (a >= dist) {
            ind += 1;
        }
        ind = -ind & 7;
        ind ^= (2 > ind) ? 1 : 0;
        uint bits = 16u + uint(i) * 3u;
        if (bits < 32u) {
            mask0 |= uint(ind) << bits;
            if (bits + 3u > 32u) {
                mask1 |= uint(ind) >> (32u - bits);
            }
        } else {
            mask1 |= uint(ind) << (bits - 32u);
        }
    }
    return uvec2(uint(hi) | (uint(lo) << 8u) | mask0, mask1);
}

// --- output ----------------------------------------------------------------

void main() {
    int texel = int(gl_FragCoord.y) * u_width + int(gl_FragCoord.x);
#ifdef MODE_BC1
    uvec4 out_blocks = uvec4(0u);
    for (int k = 0; k < 2; ++k) {
        int block = texel * 2 + k;
        if (block < u_blocks) {
            load_block(block);
            uvec2 b = bc1_block();
            if (k == 0) {
                out_blocks.xy = b;
            } else {
                out_blocks.zw = b;
            }
        }
    }
    frag_color = out_blocks;
#else
    if (texel >= u_blocks) {
        frag_color = uvec4(0u);
        return;
    }
    load_block(texel);
#ifdef MODE_BC3
    // DXT5: the alpha block first, then the color block.
    frag_color = uvec4(bc4_block(3), bc1_block());
#else
    // BC5 (RGTC2): red, then green.
    frag_color = uvec4(bc4_block(0), bc4_block(1));
#endif
#endif
}
