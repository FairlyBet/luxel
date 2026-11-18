// Block compression on the GPU for phones: ETC2 RGB (as ETC1 blocks, which
// ETC2 decodes alike), ETC2 RGBA (an EAC alpha block and an ETC color
// block) and RG11 EAC (two EAC blocks), from an RGBA8 / sRGB texture's
// level.
//
// ETC1: ported from Betsy (Matias N. Goldberg, MIT; darksylinc/betsy,
// bin/Data/etc1.glsl), itself rg-etc1 by Rich Geldreich (zlib), at its low
// quality (the subblock's average color, refined from its selectors — no
// scan around it, no precomputed solid-color tables) with four refinement
// passes; the four modes Betsy gives four threads (flip × individual /
// differential colors) tried in turn by one invocation.
//
// EAC: after etcpak (Bartosz Taudul, BSD-3; ProcessRGB.cpp,
// ProcessAlpha_ETC2): the base at the middle of the block's range, each of
// the 16 modifier tables with the multiplier that spans that range, the
// best kept. R11 takes the same 8-bit encoding (as etcpak's CompressEacR).
//
// Like bc.frag: a fragment of an RGBA32UI target encodes one 16-byte block
// or two 8-byte ones, blocks numbered row by row across the level.
//
// Defines: MODE_ETC2_RGB, MODE_ETC2_RGBA or MODE_EAC_RG; SRGB when the
// source is sRGB.

precision highp float;
precision highp int;
precision highp sampler2D;

uniform sampler2D u_source;
uniform int u_level;
uniform int u_blocks_x;
uniform int u_blocks;
uniform int u_width;

out uvec4 frag_color;

const float FLT_MAX = 3.402823466e+38;
const int REFINEMENTS = 4;
// Base colors tried per subblock: 1 (rg-etc1's low quality, the default)
// or 27 (medium: one step around the average in each channel). Measured on
// DamagedHelmet's maps, medium gains 0.6–1 dB (albedo 38.1 → 38.9 dB) for
// 2–4× the time; low is already at BC1's quality (37.5 dB) or above.
#ifndef SCAN_CANDIDATES
#define SCAN_CANDIDATES 1
#endif

vec3 srgb_encode(vec3 c) {
    c = clamp(c, 0.0, 1.0);
    return mix(c * 12.92, 1.055 * pow(c, vec3(1.0 / 2.4)) - 0.055, step(0.0031308, c));
}

// The block's 16 texels, row by row (index y * 4 + x), 0..255.
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

// --- ETC1 ------------------------------------------------------------------

vec4 inten_table(int i) {
    if (i == 0) return vec4(-8.0, -2.0, 2.0, 8.0);
    if (i == 1) return vec4(-17.0, -5.0, 5.0, 17.0);
    if (i == 2) return vec4(-29.0, -9.0, 9.0, 29.0);
    if (i == 3) return vec4(-42.0, -13.0, 13.0, 42.0);
    if (i == 4) return vec4(-60.0, -18.0, 18.0, 60.0);
    if (i == 5) return vec4(-80.0, -24.0, 24.0, 80.0);
    if (i == 6) return vec4(-106.0, -33.0, 33.0, 106.0);
    return vec4(-183.0, -47.0, 47.0, 183.0);
}

// Pixel `c` (0..7) of a subblock: two columns (flip off) or two rows (flip
// on), in Betsy's order. Returns its index in px.
int sub_pixel(bool flip, int sub, int c) {
    if (flip) {
        return (sub * 2 + (c >> 2)) * 4 + (c & 3);
    }
    return (c & 3) * 4 + sub * 2 + (c >> 2);
}

struct Solution {
    vec3 rgb; // the subblock color in 4- or 5-bit units
    int table;
    // 2 bits a pixel (c at bits 2c): its index into the table's sorted
    // values. Packed, not an array: ANGLE's HLSL mixes up arrays copied
    // inside structs (seen: pixels 5 and 7 of a subblock swapped).
    uint sel;
    float error;
};

vec3 scaled_color(vec3 rgb, bool color4) {
    float a = color4 ? 1.0 : 0.25;
    float b = color4 ? 16.0 : 8.0;
    return floor(rgb * a) + rgb * b;
}

vec3 sp[8]; // the subblock being solved

bool evaluate(vec3 rgb, bool color4, vec3 base5, bool constrain, inout Solution best) {
    if (constrain) {
        vec3 d = rgb - base5;
        if (min(d.r, min(d.g, d.b)) < -4.0 || max(d.r, max(d.g, d.b)) > 3.0) {
            return false;
        }
    }
    vec3 base = scaled_color(rgb, color4);
    Solution trial;
    trial.error = FLT_MAX;
    trial.rgb = rgb;
    trial.table = 0;
    trial.sel = 0u;
    for (int t = 0; t < 8; ++t) {
        vec4 tab = inten_table(t);
        vec3 colors[4];
        for (int s = 0; s < 4; ++s) colors[s] = clamp(base + tab[s], 0.0, 255.0);
        float total = 0.0;
        uint sel = 0u;
        for (int c = 0; c < 8; ++c) {
            float best_s = 0.0;
            vec3 d = sp[c] - colors[0];
            float best_e = dot(d, d);
            for (int s = 1; s < 4; ++s) {
                d = sp[c] - colors[s];
                float e = dot(d, d);
                if (e < best_e) {
                    best_e = e;
                    best_s = float(s);
                }
            }
            sel |= uint(best_s) << uint(2 * c);
            total += best_e;
        }
        if (total < trial.error) {
            trial.error = total;
            trial.table = t;
            trial.sel = sel;
        }
    }
    if (trial.error < best.error) {
        best = trial;
        return true;
    }
    return false;
}

// rg-etc1's optimizer: the average color (and, with SCAN_CANDIDATES 27,
// the base colors one step around it in each channel), each refined toward
// avg − (the mean of the applied deltas) — four times from the average
// itself, twice from the others.
void optimize(vec3 avg, bool color4, vec3 base5, bool constrain, inout Solution best) {
    float limit = color4 ? 15.0 : 31.0;
    vec3 avg_ls = clamp(floor(avg * limit / 255.0 + 0.5), 0.0, limit);
    for (int di = 0; di < SCAN_CANDIDATES; ++di) {
        // The average itself first (index 13 is the center of the 3×3×3).
        int k = (di + 13) % 27;
        vec3 d = vec3(float(k % 3), float((k / 3) % 3), float(k / 9)) - 1.0;
        vec3 rgb = avg_ls + d;
        if (any(lessThan(rgb, vec3(0.0))) || any(greaterThan(rgb, vec3(limit)))) {
            continue;
        }
        int trials = d == vec3(0.0) ? REFINEMENTS : 2;
        bool ok = evaluate(rgb, color4, base5, constrain, best);
        for (int r = 0; r < trials && ok; ++r) {
            vec4 tab = inten_table(best.table);
            vec3 base = scaled_color(best.rgb, color4);
            vec3 delta_sum = vec3(0.0);
            for (int c = 0; c < 8; ++c) {
                delta_sum += clamp(base + tab[int((best.sel >> uint(2 * c)) & 3u)], 0.0, 255.0) - base;
            }
            vec3 rgb1 = clamp(floor((avg - delta_sum * 0.125) * limit / 255.0 + 0.5), 0.0, limit);
            if (delta_sum == vec3(0.0) || rgb == rgb1 || best.rgb == rgb1 || avg_ls == rgb1) {
                ok = false;
            } else {
                ok = evaluate(rgb1, color4, base5, constrain, best);
            }
        }
    }
}

float selector_to_etc1(float idx) {
    return idx < 2.0 ? (3.0 - idx) : (idx - 2.0);
}

uvec2 etc1_block() {
    float best_error = FLT_MAX;
    Solution best0, best1;
    bool best_flip = false, best_color4 = true;
    for (int mode = 0; mode < 4; ++mode) {
        bool flip = (mode & 1) != 0;
        bool color4 = (mode & 2) != 0;
        Solution res[2];
        for (int s = 0; s < 2; ++s) {
            res[s].error = FLT_MAX;
            res[s].rgb = vec3(0.0);
            res[s].table = 0;
            res[s].sel = 0u;
        }
        for (int s = 0; s < 2; ++s) {
            vec3 avg = vec3(0.0);
            for (int c = 0; c < 8; ++c) {
                sp[c] = px[sub_pixel(flip, s, c)].rgb;
                avg += sp[c];
            }
            avg *= 0.125;
            optimize(avg, color4, res[0].rgb, !color4 && s == 1, res[s]);
        }
        float e = res[0].error + res[1].error;
        if (e < best_error) {
            best_error = e;
            best0 = res[0];
            best1 = res[1];
            best_flip = flip;
            best_color4 = color4;
        }
    }

    vec3 bytes;
    if (best_color4) {
        bytes = best1.rgb + best0.rgb * 16.0;
    } else {
        vec3 delta = best1.rgb - best0.rgb;
        delta += vec3(lessThan(delta, vec3(0.0))) * 8.0;
        bytes = delta + best0.rgb * 8.0;
    }
    float control = float(best1.table) * 4.0 + float(best0.table) * 32.0 +
                    (best_color4 ? 0.0 : 2.0) + (best_flip ? 1.0 : 0.0);
    uint x = uint(bytes.r) | (uint(bytes.g) << 8u) | (uint(bytes.b) << 16u) | (uint(control) << 24u);

    // Selector bits: pixel (x, y) at bit x * 4 + y; the low bits in one
    // 16-bit half, the high bits in the other, each big-endian.
    uint lsb = 0u, msb = 0u;
    for (int s = 0; s < 2; ++s) {
        for (int c = 0; c < 8; ++c) {
            int p = sub_pixel(best_flip, s, c);
            int bit = (p & 3) * 4 + (p >> 2);
            uint packed = s == 0 ? best0.sel : best1.sel;
            uint b = uint(selector_to_etc1(float((packed >> uint(2 * c)) & 3u)));
            lsb |= (b & 1u) << uint(bit);
            msb |= (b >> 1u) << uint(bit);
        }
    }
    uint y = (msb >> 8u) | ((msb & 255u) << 8u) | ((lsb >> 8u) << 16u) | ((lsb & 255u) << 24u);
    return uvec2(x, y);
}

// --- EAC -------------------------------------------------------------------

vec4 eac_lo(int t) {
    if (t == 0) return vec4(-3.0, -6.0, -9.0, -15.0);
    if (t == 1) return vec4(-3.0, -7.0, -10.0, -13.0);
    if (t == 2) return vec4(-2.0, -5.0, -8.0, -13.0);
    if (t == 3) return vec4(-2.0, -4.0, -6.0, -13.0);
    if (t == 4) return vec4(-3.0, -6.0, -8.0, -12.0);
    if (t == 5) return vec4(-3.0, -7.0, -9.0, -11.0);
    if (t == 6) return vec4(-4.0, -7.0, -8.0, -11.0);
    if (t == 7) return vec4(-3.0, -5.0, -8.0, -11.0);
    if (t == 8) return vec4(-2.0, -6.0, -8.0, -10.0);
    if (t == 9) return vec4(-2.0, -5.0, -8.0, -10.0);
    if (t == 10) return vec4(-2.0, -4.0, -8.0, -10.0);
    if (t == 11) return vec4(-2.0, -5.0, -7.0, -10.0);
    if (t == 12) return vec4(-3.0, -4.0, -7.0, -10.0);
    if (t == 13) return vec4(-1.0, -2.0, -3.0, -10.0);
    if (t == 14) return vec4(-4.0, -6.0, -8.0, -9.0);
    return vec4(-3.0, -5.0, -7.0, -9.0);
}

vec4 eac_hi(int t) {
    if (t == 0) return vec4(2.0, 5.0, 8.0, 14.0);
    if (t == 1) return vec4(2.0, 6.0, 9.0, 12.0);
    if (t == 2) return vec4(1.0, 4.0, 7.0, 12.0);
    if (t == 3) return vec4(1.0, 3.0, 5.0, 12.0);
    if (t == 4) return vec4(2.0, 5.0, 7.0, 11.0);
    if (t == 5) return vec4(2.0, 6.0, 8.0, 10.0);
    if (t == 6) return vec4(3.0, 6.0, 7.0, 10.0);
    if (t == 7) return vec4(2.0, 4.0, 7.0, 10.0);
    if (t == 8) return vec4(1.0, 5.0, 7.0, 9.0);
    if (t == 9) return vec4(1.0, 4.0, 7.0, 9.0);
    if (t == 10) return vec4(1.0, 3.0, 7.0, 9.0);
    if (t == 11) return vec4(1.0, 4.0, 6.0, 9.0);
    if (t == 12) return vec4(2.0, 3.0, 6.0, 9.0);
    if (t == 13) return vec4(0.0, 1.0, 2.0, 9.0);
    if (t == 14) return vec4(3.0, 5.0, 7.0, 8.0);
    return vec4(2.0, 4.0, 6.0, 8.0);
}

float eac_mod(int t, int j) {
    return j < 4 ? eac_lo(t)[j] : eac_hi(t)[j - 4];
}

// One channel of the block as an EAC block (8-bit values; R11 alike).
uvec2 eac_block(int channel, bool alpha) {
    // EAC numbers pixels down the columns: pixel i is at (i / 4, i % 4).
    float v[16];
    float lo = 255.0, hi = 0.0;
    for (int i = 0; i < 16; ++i) {
        v[i] = px[(i & 3) * 4 + (i >> 2)][channel];
        lo = min(lo, v[i]);
        hi = max(hi, v[i]);
    }
    if (alpha && lo == hi) {
        // ETC2 alpha: the base alone, multiplier 0.
        return uvec2(uint(lo), 0u);
    }
    float range = hi - lo;
    float mid = lo + floor(range / 2.0);
    float best_error = FLT_MAX;
    int best_t = 0;
    float best_mul = 1.0;
    for (int t = 0; t < 16; ++t) {
        // etcpak: (range * 0x100FF / (1 + table span)) >> 16, plus 1.
        float span = 1.0 + eac_hi(t)[3] - eac_lo(t)[3];
        float mul = floor(range * floor(65791.0 / span) / 65536.0) + 1.0;
        float err = 0.0;
        for (int i = 0; i < 16; ++i) {
            float e = FLT_MAX;
            for (int j = 0; j < 8; ++j) {
                float d = v[i] - clamp(mid + eac_mod(t, j) * mul, 0.0, 255.0);
                e = min(e, d * d);
            }
            err += e;
        }
        if (err < best_error) {
            best_error = err;
            best_t = t;
            best_mul = mul;
        }
    }
    uint idx_hi = 0u, idx_lo = 0u;
    for (int i = 0; i < 16; ++i) {
        float e = FLT_MAX;
        uint idx = 0u;
        for (int j = 0; j < 8; ++j) {
            float d = v[i] - clamp(mid + eac_mod(best_t, j) * best_mul, 0.0, 255.0);
            if (d * d < e) {
                e = d * d;
                idx = uint(j);
            }
        }
        if (i < 8) {
            idx_hi |= idx << uint(21 - 3 * i);
        } else {
            idx_lo |= idx << uint(21 - 3 * (i - 8));
        }
    }
    // Bytes: base, multiplier << 4 | table, then the 48 index bits big-endian.
    uint x = uint(mid) | (((uint(best_mul) << 4u) | uint(best_t)) << 8u) |
             ((idx_hi >> 16u) << 16u) | (((idx_hi >> 8u) & 255u) << 24u);
    uint y = (idx_hi & 255u) | ((idx_lo >> 16u) << 8u) | (((idx_lo >> 8u) & 255u) << 16u) |
             ((idx_lo & 255u) << 24u);
    return uvec2(x, y);
}

// --- output ----------------------------------------------------------------

void main() {
    int texel = int(gl_FragCoord.y) * u_width + int(gl_FragCoord.x);
#ifdef MODE_ETC2_RGB
    uvec4 out_blocks = uvec4(0u);
    for (int k = 0; k < 2; ++k) {
        int block = texel * 2 + k;
        if (block < u_blocks) {
            load_block(block);
            uvec2 b = etc1_block();
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
#ifdef MODE_ETC2_RGBA
    // The alpha block first, then the color block.
    frag_color = uvec4(eac_block(3, true), etc1_block());
#else
    // RG11: red, then green.
    frag_color = uvec4(eac_block(0, false), eac_block(1, false));
#endif
#endif
}
