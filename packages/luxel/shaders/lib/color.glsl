// Color encodings. Linear values are Rec.709 primaries, like glTF's.

// sRGB transfer function (IEC 61966-2-1), exact piecewise form.
vec3 linear_to_srgb(vec3 c) {
    c = clamp(c, 0.0, 1.0);
    return mix(c * 12.92, 1.055 * pow(c, vec3(1.0 / 2.4)) - 0.055, step(0.0031308, c));
}

vec3 srgb_to_linear(vec3 c) {
    return mix(c / 12.92, pow((c + 0.055) / 1.055, vec3(2.4)), step(0.04045, c));
}

// The background fill at screen position `uv` (0..1, y up), in display
// (sRGB) values: `a` rgb the color (or the gradient's start), w the kind —
// 1 a color, 2 vertical (a at the bottom, b at the top), 3 radial (a at the
// center, b at the corners); interpolated in sRGB, as CSS gradients are.
vec3 background_fill(vec2 uv, vec4 a, vec4 b) {
    if (a.w < 1.5) return a.rgb;
    float t = a.w < 2.5 ? uv.y : length(uv - 0.5) * 1.41421356;
    return mix(a.rgb, b.rgb, clamp(t, 0.0, 1.0));
}

// Relative luminance of linear Rec.709.
float luminance(vec3 c) {
    return dot(c, vec3(0.2126, 0.7152, 0.0722));
}
