// glTF data as OpenPBR inputs: everything the OpenPBR model reads
// differently for a material that came from glTF, in one place.
// Needs brdf.glsl, iridescence.glsl, surface.glsl.
//
// A material's parameters are stored as glTF's (KHR_materials_*), and
// mean what glTF means unless the material is marked OpenPBR-native
// (`setMaterialSemantics(material, "openpbr")`: OPENPBR_NATIVE), where the
// OpenPBR names carry OpenPBR's meaning as the spec gives it.
//
// For glTF data:
// - converted exactly, per pixel (`openpbr_from_gltf`): the anisotropy
//   (glTF's lobe → OpenPBR's roughness and anisotropy) and the
//   transmission's tint (glTF tints it by the base color);
// - kept as glTF means it where OpenPBR's parameters have no way to say it
//   (the functions below, each with both meanings): the specular color
//   tints F0 only, specular leaves the metal and the refraction alone, the
//   base reflects against air under a coat, a metal under a thin film is
//   Schlick from an IOR of its F0.
// Lobe shapes that differ but are not parameters (the fuzz's against
// glTF's sheen, layer order) are noted in TODO.md.

#ifndef OPENPBR_NATIVE
#define GLTF_DATA
#endif

// The stored parameters as OpenPBR's, where they convert exactly.
Surface openpbr_from_gltf(Surface s) {
#ifdef GLTF_DATA
#ifdef HAS_ANISOTROPY
    // KHR_materials_anisotropy (the Khronos renderer): α_t = mix(α, 1, a²),
    // α_b = α. OpenPBR: α_t = α' √(2 / (1 + (1 − a')²)), α_b = (1 − a') α_t.
    // Solved: 1 − a' = α_b / α_t, α' = √((α_t² + α_b²) / 2).
    float alpha = max(s.roughness * s.roughness, 1e-4);
    float alpha_t = mix(alpha, 1.0, s.anisotropy * s.anisotropy);
    s.roughness = sqrt(sqrt(0.5 * (alpha_t * alpha_t + alpha * alpha)));
    s.anisotropy = 1.0 - alpha / alpha_t;
#endif
    // glTF tints transmitted light by the base color (KHR_materials_
    // transmission); OpenPBR by transmission_color alone.
    s.transmission_tint *= s.base_color;
#endif
    return s;
}

// The IOR the base reflects with. OpenPBR: relative to the coat over it
// (the spec's TIR fix: max of the ratio and its inverse). glTF's clearcoat
// leaves the base's reflection as it is: against air (a 1.5 base under a
// 1.5 coat would lose it).
float base_reflection_ior(Surface s) {
#if defined(HAS_CLEARCOAT) && !defined(GLTF_DATA)
    float ratio = s.ior / s.coat_ior;
    return mix(s.ior, max(ratio, 1.0 / ratio), s.coat);
#else
    return s.ior;
#endif
}

// The IOR refraction bends by. OpenPBR: the base's own (light crossing a
// parallel coat bends as if from air), modulated by specular_weight. glTF:
// the IOR alone — specular "has no effect on the refraction angle"
// (KHR_materials_specular).
float refraction_ior(Surface s) {
#ifdef GLTF_DATA
    return max(s.ior, 1.0);
#else
    float xi = clamp(s.specular_weight * ior_to_f0(s.ior), 0.0, 0.99999);
    float eps = sign(s.ior - 1.0) * sqrt(xi);
    return (1.0 + eps) / (1.0 - eps);
#endif
}

// The dielectric's F0 its albedo and the layers under it see. glTF's
// specular color tints F0, clamped (KHR_materials_specular); OpenPBR's
// tints the reflection after (`dielectric_reflected`), leaving the energy
// the layers under it get alone.
vec3 dielectric_f0(float f0, vec3 color) {
#ifdef GLTF_DATA
    return min(vec3(f0) * color, vec3(1.0));
#else
    return vec3(f0);
#endif
}

// The dielectric's Fresnel from the exact one `f` (of F0 = f0). glTF: the
// tinted F0, grazing staying white — the exact curve's rise toward 1 kept:
// F' = F0' + (1 − F0') (F − f0) / (1 − f0).
vec3 dielectric_fresnel(float f, float f0, vec3 color) {
#ifdef GLTF_DATA
    vec3 f0_t = min(vec3(f0) * color, vec3(1.0));
    return f0_t + (1.0 - f0_t) * (f - f0) / max(1.0 - f0, 1e-4);
#else
    return vec3(f);
#endif
}

// What the dielectric reflects of its (film-blended) Fresnel or albedo.
// OpenPBR: × specular_color; glTF's tint is already in it.
vec3 dielectric_reflected(vec3 r, vec3 color) {
#ifdef GLTF_DATA
    return r;
#else
    return r * color;
#endif
}

// The metal's weight. OpenPBR: specular_weight; glTF's specular leaves the
// metal alone ("The metal BRDF is not affected").
float metal_weight(Surface s) {
#ifdef GLTF_DATA
    return 1.0;
#else
    return s.specular_weight;
#endif
}

// The metal's Fresnel. OpenPBR: F82-tint, the edge tinted by
// specular_color; glTF: Schlick to white.
vec3 metal_fresnel(float VdotH, vec3 f0, Surface s) {
#ifdef GLTF_DATA
    return F_Schlick(f0, vec3(1.0), VdotH);
#else
    return fresnel_f82(VdotH, f0, s.specular_color);
#endif
}

// A thin film over the metal. OpenPBR (MaterialX): over its F82 Fresnel;
// glTF (KHR_materials_iridescence, the Khronos renderer's evalIridescence):
// over Schlick from an IOR of its F0.
vec3 film_over_metal(Surface s, vec3 f0, float mu) {
#ifdef GLTF_DATA
    return openpbr_thin_film(mu, s.iridescence_ior, s.iridescence_thickness,
                             f0, vec3(1.0), false, false, 1.0);
#else
    return openpbr_thin_film(mu, s.iridescence_ior, s.iridescence_thickness,
                             f0, s.specular_color, true, false, 1.0);
#endif
}
