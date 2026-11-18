// ASTC 4×4 compression on the GPU (where the device has
// WEBGL_compressed_texture_astc: newer phones): one partition, endpoints at
// full 8-bit precision, a 4×4 weight grid (12 levels for RGB, 6 with
// alpha). Ported from niepp's astc_encoder (MIT; ASTC_Encode.hlsl,
// ASTC_IntegerSequenceEncoding.hlsl, ASTC_Table.hlsl), which takes its
// tables and block layout from ARM's astc-encoder (Apache-2.0).
//
// Changes from the original: a GLSL ES 3.00 fragment shader instead of a
// D3D compute shader, one block per fragment of an RGBA32UI target (blocks
// numbered row by row across the level, as bc.frag); only the two weight
// ranges it uses kept (QUANT_12, QUANT_6, both trits) and only plain 8-bit
// endpoints; edge blocks clamp to the level; sRGB sources encoded back to
// their stored values; a two-channel (normal map) mode that writes blue 0,
// which the PBR shader reads as "rebuild Z", where the original wrote 1.
//
// Defines: MODE_ASTC_RGB, MODE_ASTC_RGBA or MODE_ASTC_RG; SRGB when the
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

const float SMALL_VALUE = 0.00001;

vec3 srgb_encode(vec3 c) {
    c = clamp(c, 0.0, 1.0);
    return mix(c * 12.92, 1.055 * pow(c, vec3(1.0 / 2.4)) - 0.055, step(0.0031308, c));
}

// The block's 16 texels, row by row, 0..255.
vec4 texels[16];

void load_block(int block) {
    ivec2 size = textureSize(u_source, u_level);
    ivec2 base = ivec2(block % u_blocks_x, block / u_blocks_x) * 4;
    for (int i = 0; i < 16; ++i) {
        ivec2 p = min(base + ivec2(i & 3, i >> 2), size - 1);
        vec4 c = texelFetch(u_source, p, u_level);
#ifdef SRGB
        c.rgb = srgb_encode(c.rgb);
#endif
#ifdef MODE_ASTC_RG
        c.b = 0.0;
#endif
#ifndef MODE_ASTC_RGBA
        c.a = 1.0;
#endif
        texels[i] = floor(c * 255.0 + 0.5);
    }
}

// --- endpoints: the principal axis of the block's colors ------------------

vec4 eigen_vector(mat4 m) {
    vec4 v = vec4(0.26726, 0.80178, 0.53452, 0.0);
    for (int i = 0; i < 8; ++i) {
        v = m * v;
        if (length(v) < SMALL_VALUE) {
            return v;
        }
        v = normalize(m * v);
    }
    return v;
}

void principal_component_analysis(out vec4 e0, out vec4 e1) {
    vec4 mean = vec4(0.0);
    for (int i = 0; i < 16; ++i) mean += texels[i];
    mean /= 16.0;
    mat4 cov = mat4(0.0);
    for (int k = 0; k < 16; ++k) {
        vec4 t = texels[k] - mean;
        cov += outerProduct(t, t);
    }
    cov /= 15.0;
    vec4 axis = eigen_vector(cov);

    float a = 1e31, b = -1e31;
    for (int i = 0; i < 16; ++i) {
        float t = dot(texels[i] - mean, axis);
        a = min(a, t);
        b = max(b, t);
    }
    e0 = clamp(axis * a + mean, 0.0, 255.0);
    e1 = clamp(axis * b + mean, 0.0, 255.0);
    // The darker endpoint first.
    vec4 e0u = floor(e0 + 0.5), e1u = floor(e1 + 0.5);
    if (e0u.x + e0u.y + e0u.z > e1u.x + e1u.y + e1u.z) {
        vec4 t = e0;
        e0 = e1;
        e1 = t;
    }
#ifndef MODE_ASTC_RGBA
    e0.a = 255.0;
    e1.a = 255.0;
#endif
}

// --- weights ---------------------------------------------------------------

#ifdef MODE_ASTC_RGBA
const uint WEIGHT_RANGE = 5u; // QUANT_6: 0..5
const int WEIGHT_QUANT = 4;
#else
const uint WEIGHT_RANGE = 11u; // QUANT_12: 0..11
const int WEIGHT_QUANT = 7;
#endif

// ASTC's order of quantized weights (astc-encoder's scramble tables).
uint scramble(uint q) {
#ifdef MODE_ASTC_RGBA
    const uint T[6] = uint[6](0u, 2u, 4u, 5u, 3u, 1u);
#else
    const uint T[12] = uint[12](0u, 4u, 8u, 2u, 6u, 10u, 11u, 7u, 3u, 9u, 5u, 1u);
#endif
    return T[q];
}

void quantized_weights(vec4 e0, vec4 e1, out uint w[16]) {
    vec4 axis = e1 - e0;
    if (length(axis) < SMALL_VALUE) {
        for (int i = 0; i < 16; ++i) w[i] = scramble(0u);
        return;
    }
    axis = normalize(axis);
    float proj[16];
    float lo = 1e31, hi = -1e31;
    for (int i = 0; i < 16; ++i) {
        proj[i] = dot(axis, texels[i] - e0);
        lo = min(lo, proj[i]);
        hi = max(hi, proj[i]);
    }
    float inv = 1.0 / max(SMALL_VALUE, hi - lo);
    for (int i = 0; i < 16; ++i) {
        float f = (proj[i] - lo) * inv;
        uint q = uint(clamp(floor(f * float(WEIGHT_RANGE) + 0.5), 0.0, float(WEIGHT_RANGE)));
        w[i] = scramble(q);
    }
}

// --- integer sequence encoding ---------------------------------------------

const int INTEGER_FROM_TRITS[243] = int[243](
    0, 1, 2, 4, 5, 6, 8, 9, 10, 16, 17, 18, 20, 21, 22, 24, 25, 26, 3, 7, 15, 19, 23, 27, 12,
    13, 14, 32, 33, 34, 36, 37, 38, 40, 41, 42, 48, 49, 50, 52, 53, 54, 56, 57, 58, 35, 39, 47,
    51, 55, 59, 44, 45, 46, 64, 65, 66, 68, 69, 70, 72, 73, 74, 80, 81, 82, 84, 85, 86, 88, 89,
    90, 67, 71, 79, 83, 87, 91, 76, 77, 78, 128, 129, 130, 132, 133, 134, 136, 137, 138, 144,
    145, 146, 148, 149, 150, 152, 153, 154, 131, 135, 143, 147, 151, 155, 140, 141, 142, 160,
    161, 162, 164, 165, 166, 168, 169, 170, 176, 177, 178, 180, 181, 182, 184, 185, 186, 163,
    167, 175, 179, 183, 187, 172, 173, 174, 192, 193, 194, 196, 197, 198, 200, 201, 202, 208,
    209, 210, 212, 213, 214, 216, 217, 218, 195, 199, 207, 211, 215, 219, 204, 205, 206, 96,
    97, 98, 100, 101, 102, 104, 105, 106, 112, 113, 114, 116, 117, 118, 120, 121, 122, 99, 103,
    111, 115, 119, 123, 108, 109, 110, 224, 225, 226, 228, 229, 230, 232, 233, 234, 240, 241,
    242, 244, 245, 246, 248, 249, 250, 227, 231, 239, 243, 247, 251, 236, 237, 238, 28, 29, 30,
    60, 61, 62, 92, 93, 94, 156, 157, 158, 188, 189, 190, 220, 221, 222, 31, 63, 127, 159, 191,
    255, 252, 253, 254

);

// ORs the low `count` bits of `n` (at most 8) in at bit `pos`; advances it.
void put_bits(inout uvec4 out_bits, inout uint pos, uint n, uint count) {
    uint word = pos >> 5u;
    uint bit = pos & 31u;
    uint next = (pos + count) >> 5u;
    out_bits[word] |= n << bit;
    if (next > word && bit != 0u) {
        out_bits[word + 1u] |= n >> (32u - bit);
    }
    pos += count;
}

// Five values: `bits` low bits and a trit each.
void encode_trits(uint bits, uint b0, uint b1, uint b2, uint b3, uint b4,
                  inout uvec4 out_bits, inout uint pos) {
    uint t0 = b0 >> bits, t1 = b1 >> bits, t2 = b2 >> bits, t3 = b3 >> bits, t4 = b4 >> bits;
    uint mask = (1u << bits) - 1u;
    uint packed = uint(INTEGER_FROM_TRITS[int(t4 * 81u + t3 * 27u + t2 * 9u + t1 * 3u + t0)]);
    put_bits(out_bits, pos, b0 & mask, bits);
    put_bits(out_bits, pos, packed & 3u, 2u);
    put_bits(out_bits, pos, b1 & mask, bits);
    put_bits(out_bits, pos, (packed >> 2u) & 3u, 2u);
    put_bits(out_bits, pos, b2 & mask, bits);
    put_bits(out_bits, pos, (packed >> 4u) & 1u, 1u);
    put_bits(out_bits, pos, b3 & mask, bits);
    put_bits(out_bits, pos, (packed >> 5u) & 3u, 2u);
    put_bits(out_bits, pos, b4 & mask, bits);
    put_bits(out_bits, pos, (packed >> 7u) & 1u, 1u);
}

uint reverse_byte(uint p) {
    p = ((p & 0xFu) << 4u) | ((p >> 4u) & 0xFu);
    p = ((p & 0x33u) << 2u) | ((p >> 2u) & 0x33u);
    p = ((p & 0x55u) << 1u) | ((p >> 1u) & 0x55u);
    return p;
}

uint reverse_word(uint w) {
    return (reverse_byte(w & 0xFFu) << 24u) | (reverse_byte((w >> 8u) & 0xFFu) << 16u) |
           (reverse_byte((w >> 16u) & 0xFFu) << 8u) | reverse_byte(w >> 24u);
}

uvec4 encode_block() {
    vec4 e0, e1;
    principal_component_analysis(e0, e1);

    // Endpoints, 8 bits each: r0 r1 g0 g1 b0 b1 (a0 a1).
    uvec4 q0 = uvec4(floor(e0 + 0.5)), q1 = uvec4(floor(e1 + 0.5));
    uvec4 ep = uvec4(0u);
    uint pos = 0u;
    put_bits(ep, pos, q0.r, 8u);
    put_bits(ep, pos, q1.r, 8u);
    put_bits(ep, pos, q0.g, 8u);
    put_bits(ep, pos, q1.g, 8u);
    put_bits(ep, pos, q0.b, 8u);
    put_bits(ep, pos, q1.b, 8u);
#ifdef MODE_ASTC_RGBA
    put_bits(ep, pos, q0.a, 8u);
    put_bits(ep, pos, q1.a, 8u);
#endif

    // Weights: trits and 2 (QUANT_12) or 1 (QUANT_6) bits each.
    uint w[16];
    quantized_weights(e0, e1, w);
#ifdef MODE_ASTC_RGBA
    uint wbits = 1u;
#else
    uint wbits = 2u;
#endif
    uvec4 wt = uvec4(0u);
    pos = 0u;
    encode_trits(wbits, w[0], w[1], w[2], w[3], w[4], wt, pos);
    encode_trits(wbits, w[5], w[6], w[7], w[8], w[9], wt, pos);
    encode_trits(wbits, w[10], w[11], w[12], w[13], w[14], wt, pos);
    encode_trits(wbits, w[15], 0u, 0u, 0u, 0u, wt, pos);

    // Block mode (Table C.2.8, first row): a 4×4 weight grid (A = 2,
    // B = 0), one plane, the weight range in R and H (Table C.2.7).
    uint h = WEIGHT_QUANT < 6 ? 0u : 1u;
    uint r = uint(WEIGHT_QUANT % 6) + 2u;
    uint mode = ((r >> 1u) & 3u) | ((r & 1u) << 4u) | (2u << 5u) | (h << 9u);
#ifdef MODE_ASTC_RGBA
    uint cem = 12u; // LDR RGBA direct
#else
    uint cem = 8u; // LDR RGB direct
#endif

    // The weights fill the block from its top end, bit-reversed.
    uvec4 blk = uvec4(0u);
    blk.w = reverse_word(wt.x);
    blk.z = reverse_word(wt.y);
    blk.y = reverse_word(wt.z);
    blk.x = mode | (cem << 13u) | ((ep.x & 0x7FFFu) << 17u);
    blk.y |= ((ep.x >> 15u) & 0x1FFFFu) | ((ep.y & 0x7FFFu) << 17u);
    blk.z |= (ep.y >> 15u) & 0x1FFFFu;
    return blk;
}

void main() {
    int texel = int(gl_FragCoord.y) * u_width + int(gl_FragCoord.x);
    if (texel >= u_blocks) {
        frag_color = uvec4(0u);
        return;
    }
    load_block(texel);
    frag_color = encode_block();
}
