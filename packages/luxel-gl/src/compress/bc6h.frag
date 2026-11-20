// BC6H (unsigned half floats) compression on the GPU, for HDR textures —
// the environment's cube maps. Ported from Godot's modules/betsy/bc6h.glsl
// (MIT), itself Krzysztof Narkowicz's GPURealTimeBC6H (public domain) with
// Matias N. Goldberg's changes: mode 11 (one region, 10-bit endpoints)
// against the best of the 32 two-region patterns in modes 2 and 6 (its
// "quality" preset), scored by mean squared log error.
//
// Changes from the original: a GLSL ES 3.00 fragment shader instead of a
// compute shader (one block per fragment of an RGBA32UI target, blocks
// numbered row by row across the level, as bc.frag), explicit int / float
// conversions; the unsigned path only; texels read with texelFetch (a 2D
// source) or, for a cube face (CUBE), along the direction to each texel's
// center at an exact level with nearest filtering, instead of textureGather.

precision highp float;
precision highp int;
precision highp sampler2D;
precision highp samplerCube;

#ifdef CUBE
uniform samplerCube u_source;
uniform int u_face; // +X −X +Y −Y +Z −Z
#else
uniform sampler2D u_source;
#endif
uniform int u_level;
uniform int u_blocks_x;
uniform int u_blocks;
uniform int u_width;

out uvec4 frag_color;

const float HALF_MAX = 65504.0;
const float HALF_MIN = 0.0;
const uint PATTERN_NUM = 32u;

vec3 texels[16];

#ifdef CUBE
// The direction to texel (x, y)'s center on face u_face, GL's cube layout.
vec3 face_dir(ivec2 p, int size) {
    vec2 st = (vec2(p) + 0.5) / float(size) * 2.0 - 1.0;
    if (u_face == 0) return vec3(1.0, -st.y, -st.x);
    if (u_face == 1) return vec3(-1.0, -st.y, st.x);
    if (u_face == 2) return vec3(st.x, 1.0, st.y);
    if (u_face == 3) return vec3(st.x, -1.0, -st.y);
    if (u_face == 4) return vec3(st.x, -st.y, 1.0);
    return vec3(-st.x, -st.y, -1.0);
}
#endif

void load_block(int block) {
    ivec2 size = textureSize(u_source, u_level);
    ivec2 base = ivec2(block % u_blocks_x, block / u_blocks_x) * 4;
    for (int i = 0; i < 16; ++i) {
        ivec2 p = min(base + ivec2(i & 3, i >> 2), size - 1);
#ifdef CUBE
        vec3 c = textureLod(u_source, face_dir(p, size.x), float(u_level)).rgb;
#else
        vec3 c = texelFetch(u_source, p, u_level).rgb;
#endif
        texels[i] = clamp(c, 0.0, HALF_MAX);
    }
}

vec3 f32tof16(vec3 v) {
    return vec3(float(packHalf2x16(vec2(v.x, 0.0))), float(packHalf2x16(vec2(v.y, 0.0))),
                float(packHalf2x16(vec2(v.z, 0.0))));
}

float f32tof16(float v) {
    return float(packHalf2x16(vec2(v, 0.0)));
}

vec3 f16tof32(uvec3 v) {
    return vec3(unpackHalf2x16(v.x).x, unpackHalf2x16(v.y).x, unpackHalf2x16(v.z).x);
}

float CalcMSLE(vec3 a, vec3 b) {
    vec3 err = log2((b + 1.0) / (a + 1.0));
    err = err * err;
    return err.x + err.y + err.z;
}

vec3 customLog2(vec3 a) {
    return log2(a + 1.0);
}

vec3 customExp2(vec3 a) {
    return exp2(a) - 1.0;
}

uint PatternFixupID(uint i) {
    uint ret = 15u;
    ret = ((3441033216u >> i) & 0x1u) != 0u ? 2u : ret;
    ret = ((845414400u >> i) & 0x1u) != 0u ? 8u : ret;
    return ret;
}

uint Pattern(uint p, uint i) {
    uint p2 = p / 2u;
    uint p3 = p - p2 * 2u;
    uint enc = 0u;
    enc = p2 == 0u ? 2290666700u : enc;
    enc = p2 == 1u ? 3972591342u : enc;
    enc = p2 == 2u ? 4276930688u : enc;
    enc = p2 == 3u ? 3967876808u : enc;
    enc = p2 == 4u ? 4293707776u : enc;
    enc = p2 == 5u ? 3892379264u : enc;
    enc = p2 == 6u ? 4278255592u : enc;
    enc = p2 == 7u ? 4026597360u : enc;
    enc = p2 == 8u ? 9369360u : enc;
    enc = p2 == 9u ? 147747072u : enc;
    enc = p2 == 10u ? 1930428556u : enc;
    enc = p2 == 11u ? 2362323200u : enc;
    enc = p2 == 12u ? 823134348u : enc;
    enc = p2 == 13u ? 913073766u : enc;
    enc = p2 == 14u ? 267393000u : enc;
    enc = p2 == 15u ? 966553998u : enc;
    enc = p3 != 0u ? enc >> 16u : enc;
    return (enc >> i) & 0x1u;
}

vec3 Quantize7(vec3 x) {
    return (f32tof16(x) * 128.0) / 31744.0;
}

vec3 Quantize9(vec3 x) {
    return (f32tof16(x) * 512.0) / 31744.0;
}

vec3 Quantize10(vec3 x) {
    return (f32tof16(x) * 1024.0) / 31744.0;
}

vec3 Unquantize7(vec3 x) {
    return (x * 65536.0 + 32768.0) / 128.0;
}

vec3 Unquantize9(vec3 x) {
    return (x * 65536.0 + 32768.0) / 512.0;
}

vec3 Unquantize10(vec3 x) {
    return (x * 65536.0 + 32768.0) / 1024.0;
}

vec3 FinishUnquantize(vec3 endpoint0Unq, vec3 endpoint1Unq, float weight) {
    vec3 comp = (endpoint0Unq * (64.0 - weight) + endpoint1Unq * weight + 32.0) * (31.0 / 4096.0);
    return f16tof32(uvec3(comp));
}

void Swap(inout vec3 a, inout vec3 b) {
    vec3 tmp = a;
    a = b;
    b = tmp;
}

void Swap(inout float a, inout float b) {
    float tmp = a;
    a = b;
    b = tmp;
}

// Flat (both endpoints equal): index 0, not 0 / 0.
uint ComputeIndex3(float texelPos, float endPoint0Pos, float endPoint1Pos) {
    if (endPoint1Pos == endPoint0Pos) return 0u;
    float r = (texelPos - endPoint0Pos) / (endPoint1Pos - endPoint0Pos);
    return uint(clamp(r * 6.98182 + 0.00909 + 0.5, 0.0, 7.0));
}

uint ComputeIndex4(float texelPos, float endPoint0Pos, float endPoint1Pos) {
    if (endPoint1Pos == endPoint0Pos) return 0u;
    float r = (texelPos - endPoint0Pos) / (endPoint1Pos - endPoint0Pos);
    return uint(clamp(r * 14.93333 + 0.03333 + 0.5, 0.0, 15.0));
}

// Adds the sign bit to quantized deltas.
void SignExtend(inout vec3 v1, uint mask, uint signFlag) {
    ivec3 v = ivec3(v1);
    v.x = (v.x & int(mask)) | (v.x < 0 ? int(signFlag) : 0);
    v.y = (v.y & int(mask)) | (v.y < 0 ? int(signFlag) : 0);
    v.z = (v.z & int(mask)) | (v.z < 0 ? int(signFlag) : 0);
    v1 = vec3(v);
}

// Mode 11: one region, two 10-bit endpoints.
void EncodeP1(inout uvec4 block, inout float blockMSLE) {
    vec3 blockMin = texels[0];
    vec3 blockMax = texels[0];
    for (int i = 1; i < 16; ++i) {
        blockMin = min(blockMin, texels[i]);
        blockMax = max(blockMax, texels[i]);
    }

    // Refine the endpoints in log2 RGB space.
    vec3 refinedBlockMin = blockMax;
    vec3 refinedBlockMax = blockMin;
    for (int i = 0; i < 16; ++i) {
        refinedBlockMin = min(refinedBlockMin, texels[i] == blockMin ? refinedBlockMin : texels[i]);
        refinedBlockMax = max(refinedBlockMax, texels[i] == blockMax ? refinedBlockMax : texels[i]);
    }

    vec3 logBlockMax = customLog2(blockMax);
    vec3 logBlockMin = customLog2(blockMin);
    vec3 logRefinedBlockMax = customLog2(refinedBlockMax);
    vec3 logRefinedBlockMin = customLog2(refinedBlockMin);
    vec3 logBlockMaxExt = (logBlockMax - logBlockMin) * (1.0 / 32.0);

    logBlockMin += min(logRefinedBlockMin - logBlockMin, logBlockMaxExt);
    logBlockMax -= min(logBlockMax - logRefinedBlockMax, logBlockMaxExt);
    blockMin = customExp2(logBlockMin);
    blockMax = customExp2(logBlockMax);

    vec3 blockDir = blockMax - blockMin;
    blockDir = blockDir / max(blockDir.x + blockDir.y + blockDir.z, 1e-20); // (flat: 0)

    vec3 endpoint0 = Quantize10(blockMin);
    vec3 endpoint1 = Quantize10(blockMax);
    float endPoint0Pos = f32tof16(dot(blockMin, blockDir));
    float endPoint1Pos = f32tof16(dot(blockMax, blockDir));

    // Swap the endpoints if the first texel's index would need its high bit.
    float fixupTexelPos = f32tof16(dot(texels[0], blockDir));
    uint fixupIndex = ComputeIndex4(fixupTexelPos, endPoint0Pos, endPoint1Pos);
    if (fixupIndex > 7u) {
        Swap(endPoint0Pos, endPoint1Pos);
        Swap(endpoint0, endpoint1);
    }

    uint indices[16];
    for (int i = 0; i < 16; ++i) {
        float texelPos = f32tof16(dot(texels[i], blockDir));
        indices[i] = ComputeIndex4(texelPos, endPoint0Pos, endPoint1Pos);
    }

    vec3 endpoint0Unq = Unquantize10(endpoint0);
    vec3 endpoint1Unq = Unquantize10(endpoint1);
    float msle = 0.0;
    for (int i = 0; i < 16; ++i) {
        float weight = floor((float(indices[i]) * 64.0) / 15.0 + 0.5);
        vec3 texelUnc = FinishUnquantize(endpoint0Unq, endpoint1Unq, weight);
        msle += CalcMSLE(texels[i], texelUnc);
    }

    blockMSLE = msle;
    block.x = 0x03u;

    block.x |= uint(endpoint0.x) << 5u;
    block.x |= uint(endpoint0.y) << 15u;
    block.x |= uint(endpoint0.z) << 25u;
    block.y |= uint(endpoint0.z) >> 7u;
    block.y |= uint(endpoint1.x) << 3u;
    block.y |= uint(endpoint1.y) << 13u;
    block.y |= uint(endpoint1.z) << 23u;
    block.z |= uint(endpoint1.z) >> 9u;

    block.z |= indices[0] << 1u;
    block.z |= indices[1] << 4u;
    block.z |= indices[2] << 8u;
    block.z |= indices[3] << 12u;
    block.z |= indices[4] << 16u;
    block.z |= indices[5] << 20u;
    block.z |= indices[6] << 24u;
    block.z |= indices[7] << 28u;
    block.w |= indices[8] << 0u;
    block.w |= indices[9] << 4u;
    block.w |= indices[10] << 8u;
    block.w |= indices[11] << 12u;
    block.w |= indices[12] << 16u;
    block.w |= indices[13] << 20u;
    block.w |= indices[14] << 24u;
    block.w |= indices[15] << 28u;
}

float DistToLineSq(vec3 PointOnLine, vec3 LineDirection, vec3 Point) {
    vec3 w = Point - PointOnLine;
    vec3 x = w - dot(w, LineDirection) * LineDirection;
    return dot(x, x);
}

// How far a pattern's two regions lie from their lines (smaller is better).
float EvaluateP2Pattern(uint pattern) {
    vec3 p0BlockMin = vec3(HALF_MAX);
    vec3 p0BlockMax = vec3(HALF_MIN);
    vec3 p1BlockMin = vec3(HALF_MAX);
    vec3 p1BlockMax = vec3(HALF_MIN);
    for (uint i = 0u; i < 16u; ++i) {
        if (Pattern(pattern, i) == 0u) {
            p0BlockMin = min(p0BlockMin, texels[i]);
            p0BlockMax = max(p0BlockMax, texels[i]);
        } else {
            p1BlockMin = min(p1BlockMin, texels[i]);
            p1BlockMax = max(p1BlockMax, texels[i]);
        }
    }
    // (A flat partition has no direction: 0, not normalize's NaN.)
    vec3 p0d = p0BlockMax - p0BlockMin;
    vec3 p1d = p1BlockMax - p1BlockMin;
    vec3 p0BlockDir = dot(p0d, p0d) > 0.0 ? normalize(p0d) : vec3(0.0);
    vec3 p1BlockDir = dot(p1d, p1d) > 0.0 ? normalize(p1d) : vec3(0.0);
    float sqDistanceFromLine = 0.0;
    for (uint i = 0u; i < 16u; ++i) {
        if (Pattern(pattern, i) == 0u) {
            sqDistanceFromLine += DistToLineSq(p0BlockMin, p0BlockDir, texels[i]);
        } else {
            sqDistanceFromLine += DistToLineSq(p1BlockMin, p1BlockDir, texels[i]);
        }
    }
    return sqDistanceFromLine;
}

// Mode 2 (7-bit base, 6-bit deltas) or mode 6 (9-bit base, 5-bit deltas),
// whichever errs less, for a two-region pattern; kept if better than the
// block so far.
void EncodeP2Pattern(inout uvec4 block, inout float blockMSLE, uint pattern) {
    vec3 p0BlockMin = vec3(HALF_MAX);
    vec3 p0BlockMax = vec3(HALF_MIN);
    vec3 p1BlockMin = vec3(HALF_MAX);
    vec3 p1BlockMax = vec3(HALF_MIN);
    for (uint i = 0u; i < 16u; ++i) {
        if (Pattern(pattern, i) == 0u) {
            p0BlockMin = min(p0BlockMin, texels[i]);
            p0BlockMax = max(p0BlockMax, texels[i]);
        } else {
            p1BlockMin = min(p1BlockMin, texels[i]);
            p1BlockMax = max(p1BlockMax, texels[i]);
        }
    }

    vec3 p0BlockDir = p0BlockMax - p0BlockMin;
    vec3 p1BlockDir = p1BlockMax - p1BlockMin;
    p0BlockDir = p0BlockDir / max(p0BlockDir.x + p0BlockDir.y + p0BlockDir.z, 1e-20);
    p1BlockDir = p1BlockDir / max(p1BlockDir.x + p1BlockDir.y + p1BlockDir.z, 1e-20);

    float p0Endpoint0Pos = f32tof16(dot(p0BlockMin, p0BlockDir));
    float p0Endpoint1Pos = f32tof16(dot(p0BlockMax, p0BlockDir));
    float p1Endpoint0Pos = f32tof16(dot(p1BlockMin, p1BlockDir));
    float p1Endpoint1Pos = f32tof16(dot(p1BlockMax, p1BlockDir));

    uint fixupID = PatternFixupID(pattern);
    float p0FixupTexelPos = f32tof16(dot(texels[0], p0BlockDir));
    float p1FixupTexelPos = f32tof16(dot(texels[fixupID], p1BlockDir));
    uint p0FixupIndex = ComputeIndex3(p0FixupTexelPos, p0Endpoint0Pos, p0Endpoint1Pos);
    uint p1FixupIndex = ComputeIndex3(p1FixupTexelPos, p1Endpoint0Pos, p1Endpoint1Pos);
    if (p0FixupIndex > 3u) {
        Swap(p0Endpoint0Pos, p0Endpoint1Pos);
        Swap(p0BlockMin, p0BlockMax);
    }
    if (p1FixupIndex > 3u) {
        Swap(p1Endpoint0Pos, p1Endpoint1Pos);
        Swap(p1BlockMin, p1BlockMax);
    }

    uint indices[16];
    for (uint i = 0u; i < 16u; ++i) {
        float p0TexelPos = f32tof16(dot(texels[i], p0BlockDir));
        float p1TexelPos = f32tof16(dot(texels[i], p1BlockDir));
        uint p0Index = ComputeIndex3(p0TexelPos, p0Endpoint0Pos, p0Endpoint1Pos);
        uint p1Index = ComputeIndex3(p1TexelPos, p1Endpoint0Pos, p1Endpoint1Pos);
        indices[i] = Pattern(pattern, i) == 0u ? p0Index : p1Index;
    }

    vec3 endpoint760 = floor(Quantize7(p0BlockMin));
    vec3 endpoint761 = floor(Quantize7(p0BlockMax));
    vec3 endpoint762 = floor(Quantize7(p1BlockMin));
    vec3 endpoint763 = floor(Quantize7(p1BlockMax));

    vec3 endpoint950 = floor(Quantize9(p0BlockMin));
    vec3 endpoint951 = floor(Quantize9(p0BlockMax));
    vec3 endpoint952 = floor(Quantize9(p1BlockMin));
    vec3 endpoint953 = floor(Quantize9(p1BlockMax));

    endpoint761 = endpoint761 - endpoint760;
    endpoint762 = endpoint762 - endpoint760;
    endpoint763 = endpoint763 - endpoint760;

    endpoint951 = endpoint951 - endpoint950;
    endpoint952 = endpoint952 - endpoint950;
    endpoint953 = endpoint953 - endpoint950;

    endpoint761 = clamp(endpoint761, -31.0, 31.0);
    endpoint762 = clamp(endpoint762, -31.0, 31.0);
    endpoint763 = clamp(endpoint763, -31.0, 31.0);

    endpoint951 = clamp(endpoint951, -15.0, 15.0);
    endpoint952 = clamp(endpoint952, -15.0, 15.0);
    endpoint953 = clamp(endpoint953, -15.0, 15.0);

    vec3 endpoint760Unq = Unquantize7(endpoint760);
    vec3 endpoint761Unq = Unquantize7(endpoint760 + endpoint761);
    vec3 endpoint762Unq = Unquantize7(endpoint760 + endpoint762);
    vec3 endpoint763Unq = Unquantize7(endpoint760 + endpoint763);
    vec3 endpoint950Unq = Unquantize9(endpoint950);
    vec3 endpoint951Unq = Unquantize9(endpoint950 + endpoint951);
    vec3 endpoint952Unq = Unquantize9(endpoint950 + endpoint952);
    vec3 endpoint953Unq = Unquantize9(endpoint950 + endpoint953);

    float msle76 = 0.0;
    float msle95 = 0.0;
    for (uint i = 0u; i < 16u; ++i) {
        bool p0 = Pattern(pattern, i) == 0u;
        vec3 tmp760Unq = p0 ? endpoint760Unq : endpoint762Unq;
        vec3 tmp761Unq = p0 ? endpoint761Unq : endpoint763Unq;
        vec3 tmp950Unq = p0 ? endpoint950Unq : endpoint952Unq;
        vec3 tmp951Unq = p0 ? endpoint951Unq : endpoint953Unq;
        float weight = floor((float(indices[i]) * 64.0) / 7.0 + 0.5);
        msle76 += CalcMSLE(texels[i], FinishUnquantize(tmp760Unq, tmp761Unq, weight));
        msle95 += CalcMSLE(texels[i], FinishUnquantize(tmp950Unq, tmp951Unq, weight));
    }

    SignExtend(endpoint761, 0x1Fu, 0x20u);
    SignExtend(endpoint762, 0x1Fu, 0x20u);
    SignExtend(endpoint763, 0x1Fu, 0x20u);

    SignExtend(endpoint951, 0xFu, 0x10u);
    SignExtend(endpoint952, 0xFu, 0x10u);
    SignExtend(endpoint953, 0xFu, 0x10u);

    float p2MSLE = min(msle76, msle95);
    if (p2MSLE < blockMSLE) {
        blockMSLE = p2MSLE;
        block = uvec4(0u);

        if (p2MSLE == msle76) {
            // 7.6
            block.x = 0x1u;
            block.x |= (uint(endpoint762.y) & 0x20u) >> 3u;
            block.x |= (uint(endpoint763.y) & 0x10u) >> 1u;
            block.x |= (uint(endpoint763.y) & 0x20u) >> 1u;
            block.x |= uint(endpoint760.x) << 5u;
            block.x |= (uint(endpoint763.z) & 0x01u) << 12u;
            block.x |= (uint(endpoint763.z) & 0x02u) << 12u;
            block.x |= (uint(endpoint762.z) & 0x10u) << 10u;
            block.x |= uint(endpoint760.y) << 15u;
            block.x |= (uint(endpoint762.z) & 0x20u) << 17u;
            block.x |= (uint(endpoint763.z) & 0x04u) << 21u;
            block.x |= (uint(endpoint762.y) & 0x10u) << 20u;
            block.x |= uint(endpoint760.z) << 25u;
            block.y |= (uint(endpoint763.z) & 0x08u) >> 3u;
            block.y |= (uint(endpoint763.z) & 0x20u) >> 4u;
            block.y |= (uint(endpoint763.z) & 0x10u) >> 2u;
            block.y |= uint(endpoint761.x) << 3u;
            block.y |= (uint(endpoint762.y) & 0x0Fu) << 9u;
            block.y |= uint(endpoint761.y) << 13u;
            block.y |= (uint(endpoint763.y) & 0x0Fu) << 19u;
            block.y |= uint(endpoint761.z) << 23u;
            block.y |= (uint(endpoint762.z) & 0x07u) << 29u;
            block.z |= (uint(endpoint762.z) & 0x08u) >> 3u;
            block.z |= uint(endpoint762.x) << 1u;
            block.z |= uint(endpoint763.x) << 7u;
        } else {
            // 9.5
            block.x = 0xEu;
            block.x |= uint(endpoint950.x) << 5u;
            block.x |= (uint(endpoint952.z) & 0x10u) << 10u;
            block.x |= uint(endpoint950.y) << 15u;
            block.x |= (uint(endpoint952.y) & 0x10u) << 20u;
            block.x |= uint(endpoint950.z) << 25u;
            block.y |= uint(endpoint950.z) >> 7u;
            block.y |= (uint(endpoint953.z) & 0x10u) >> 2u;
            block.y |= uint(endpoint951.x) << 3u;
            block.y |= (uint(endpoint953.y) & 0x10u) << 4u;
            block.y |= (uint(endpoint952.y) & 0x0Fu) << 9u;
            block.y |= uint(endpoint951.y) << 13u;
            block.y |= (uint(endpoint953.z) & 0x01u) << 18u;
            block.y |= (uint(endpoint953.y) & 0x0Fu) << 19u;
            block.y |= uint(endpoint951.z) << 23u;
            block.y |= (uint(endpoint953.z) & 0x02u) << 27u;
            block.y |= uint(endpoint952.z) << 29u;
            block.z |= (uint(endpoint952.z) & 0x08u) >> 3u;
            block.z |= uint(endpoint952.x) << 1u;
            block.z |= (uint(endpoint953.z) & 0x04u) << 4u;
            block.z |= uint(endpoint953.x) << 7u;
            block.z |= (uint(endpoint953.z) & 0x08u) << 9u;
        }

        block.z |= pattern << 13u;
        uint blockFixupID = PatternFixupID(pattern);
        if (blockFixupID == 15u) {
            block.z |= indices[0] << 18u;
            block.z |= indices[1] << 20u;
            block.z |= indices[2] << 23u;
            block.z |= indices[3] << 26u;
            block.z |= indices[4] << 29u;
            block.w |= indices[5] << 0u;
            block.w |= indices[6] << 3u;
            block.w |= indices[7] << 6u;
            block.w |= indices[8] << 9u;
            block.w |= indices[9] << 12u;
            block.w |= indices[10] << 15u;
            block.w |= indices[11] << 18u;
            block.w |= indices[12] << 21u;
            block.w |= indices[13] << 24u;
            block.w |= indices[14] << 27u;
            block.w |= indices[15] << 30u;
        } else if (blockFixupID == 2u) {
            block.z |= indices[0] << 18u;
            block.z |= indices[1] << 20u;
            block.z |= indices[2] << 23u;
            block.z |= indices[3] << 25u;
            block.z |= indices[4] << 28u;
            block.z |= indices[5] << 31u;
            block.w |= indices[5] >> 1u;
            block.w |= indices[6] << 2u;
            block.w |= indices[7] << 5u;
            block.w |= indices[8] << 8u;
            block.w |= indices[9] << 11u;
            block.w |= indices[10] << 14u;
            block.w |= indices[11] << 17u;
            block.w |= indices[12] << 20u;
            block.w |= indices[13] << 23u;
            block.w |= indices[14] << 26u;
            block.w |= indices[15] << 29u;
        } else {
            block.z |= indices[0] << 18u;
            block.z |= indices[1] << 20u;
            block.z |= indices[2] << 23u;
            block.z |= indices[3] << 26u;
            block.z |= indices[4] << 29u;
            block.w |= indices[5] << 0u;
            block.w |= indices[6] << 3u;
            block.w |= indices[7] << 6u;
            block.w |= indices[8] << 9u;
            block.w |= indices[9] << 11u;
            block.w |= indices[10] << 14u;
            block.w |= indices[11] << 17u;
            block.w |= indices[12] << 20u;
            block.w |= indices[13] << 23u;
            block.w |= indices[14] << 26u;
            block.w |= indices[15] << 29u;
        }
    }
}

void main() {
    int texel = int(gl_FragCoord.y) * u_width + int(gl_FragCoord.x);
    if (texel >= u_blocks) {
        frag_color = uvec4(0u);
        return;
    }
    load_block(texel);
    uvec4 block = uvec4(0u);
    float blockMSLE = 0.0;
    EncodeP1(block, blockMSLE);
    float bestScore = EvaluateP2Pattern(0u);
    uint bestPattern = 0u;
    for (uint i = 1u; i < PATTERN_NUM; ++i) {
        float score = EvaluateP2Pattern(i);
        if (score < bestScore) {
            bestPattern = i;
            bestScore = score;
        }
    }
    EncodeP2Pattern(block, blockMSLE, bestPattern);
    frag_color = block;
}
