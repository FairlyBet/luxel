//! OpenPBR Surface parameter names (`base_color`, `coat_weight`, …) for the
//! material API, next to glTF's (`baseColorFactor`, `clearcoatFactor`, …).
//! Materials store glTF's parameters plus OpenPBR's own; each OpenPBR name
//! reads and writes the ones that carry it. The values are stored as given;
//! what they mean to the OpenPBR model — glTF's (a loaded material) or
//! OpenPBR's — is the material's `setMaterialSemantics`:
//!
//! | OpenPBR | stored as |
//! |---|---|
//! | base_weight, base_diffuse_roughness | baseWeight, diffuseRoughness |
//! | base_color, geometry_opacity | baseColorFactor rgb, a |
//! | base_metalness | metallicFactor |
//! | specular_weight, specular_color, specular_roughness, specular_ior | specularFactor, specularColorFactor, roughnessFactor, ior |
//! | specular_roughness_anisotropy | anisotropyStrength |
//! | transmission_weight, transmission_depth | transmissionFactor, attenuationDistance (0: no medium) |
//! | transmission_color | attenuationColor with a depth (absorbed in the medium), transmissionTint without (a tint on the surface, as OpenPBR) |
//! | transmission_dispersion_abbe_number | dispersion = 20 / Abbe |
//! | coat_weight, coat_roughness, coat_ior, coat_color, coat_darkening | clearcoatFactor, clearcoatRoughnessFactor, coatIor, coatColor, coatDarkening |
//! | coat_roughness_anisotropy | coatAnisotropy |
//! | subsurface_weight, subsurface_color, subsurface_radius, subsurface_radius_scale | subsurfaceWeight, subsurfaceColor, subsurfaceRadius, subsurfaceRadiusScale |
//! | fuzz_weight, fuzz_color, fuzz_roughness | sheenColorFactor = weight × color, sheenRoughnessFactor |
//! | thin_film_weight, thin_film_thickness (µm), thin_film_ior | iridescenceFactor, iridescenceThicknessMaximum (nm), iridescenceIor |
//! | emission_luminance, emission_color | emissiveStrength, emissiveFactor |
//!
//! Emission is radiance in the scene's units, as glTF's (not photometric
//! nits). A volume's depth needs `thicknessFactor` > 0 to absorb (glTF's
//! convention). Subsurface is MaterialX's real-time approximation (no
//! scatter anisotropy, not thin-walled).

use luxel_gltf::Material;

/// Stored properties to write: (glTF-side name, values).
pub type Writes = Vec<(&'static str, Vec<f32>)>;

/// Every OpenPBR name the API takes.
pub const NAMES: [&str; 32] = [
    "base_weight",
    "base_color",
    "base_metalness",
    "base_diffuse_roughness",
    "specular_weight",
    "specular_color",
    "specular_roughness",
    "specular_ior",
    "specular_roughness_anisotropy",
    "transmission_weight",
    "transmission_color",
    "transmission_depth",
    "transmission_dispersion_abbe_number",
    "coat_weight",
    "coat_color",
    "coat_roughness",
    "coat_ior",
    "coat_darkening",
    "coat_roughness_anisotropy",
    "subsurface_weight",
    "subsurface_color",
    "subsurface_radius",
    "subsurface_radius_scale",
    "fuzz_weight",
    "fuzz_color",
    "fuzz_roughness",
    "thin_film_weight",
    "thin_film_thickness",
    "thin_film_ior",
    "emission_luminance",
    "emission_color",
    "geometry_opacity",
];

// Names that are one stored property as is.
const DIRECT: [(&str, &str); 21] = [
    ("base_weight", "baseWeight"),
    ("base_metalness", "metallicFactor"),
    ("base_diffuse_roughness", "diffuseRoughness"),
    ("specular_weight", "specularFactor"),
    ("specular_color", "specularColorFactor"),
    ("specular_roughness", "roughnessFactor"),
    ("specular_ior", "ior"),
    ("specular_roughness_anisotropy", "anisotropyStrength"),
    ("transmission_weight", "transmissionFactor"),
    ("coat_weight", "clearcoatFactor"),
    ("coat_color", "coatColor"),
    ("coat_roughness", "clearcoatRoughnessFactor"),
    ("coat_ior", "coatIor"),
    ("coat_darkening", "coatDarkening"),
    ("coat_roughness_anisotropy", "coatAnisotropy"),
    ("subsurface_weight", "subsurfaceWeight"),
    ("subsurface_color", "subsurfaceColor"),
    ("subsurface_radius", "subsurfaceRadius"),
    ("subsurface_radius_scale", "subsurfaceRadiusScale"),
    ("fuzz_roughness", "sheenRoughnessFactor"),
    ("thin_film_weight", "iridescenceFactor"),
];

fn prop(m: &Material, name: &str) -> Vec<f32> {
    m.property(name).unwrap_or_default()
}

/// OpenPBR's fuzz as (weight, color). glTF stores only their product (the
/// sheen color), which can't be split back: the pair as last set is kept
/// beside it and used while the sheen color still equals its product.
pub type Fuzz = (f32, [f32; 3]);

/// The fuzz behind `m`'s sheen color: `last` while it still matches,
/// otherwise (a loaded material, or `sheenColorFactor` set by its glTF name)
/// split as weight = the brightest channel, color = sheen / weight.
pub fn fuzz(m: &Material, last: Fuzz) -> Fuzz {
    let sheen = prop(m, "sheenColorFactor");
    let (w, c) = last;
    if sheen.len() == 3 && (0..3).all(|i| (sheen[i] - w * c[i]).abs() <= 1e-6) {
        return last;
    }
    let w = sheen.iter().copied().fold(0.0, f32::max);
    let color = if w > 0.0 && sheen.len() == 3 {
        [sheen[0] / w, sheen[1] / w, sheen[2] / w]
    } else {
        c
    };
    (w, color)
}

/// The value of OpenPBR parameter `name`; None if it isn't one.
pub fn get(m: &Material, name: &str, last_fuzz: Fuzz) -> Option<Vec<f32>> {
    if let Some((_, p)) = DIRECT.iter().find(|(n, _)| *n == name) {
        return Some(prop(m, p));
    }
    let base = prop(m, "baseColorFactor");
    Some(match name {
        // transmission_color: absorbed over transmission_depth in a medium
        // (glTF's attenuationColor), or, depth 0 (no medium), a tint on the
        // surface (transmissionTint).
        "transmission_depth" => prop(m, "attenuationDistance"),
        "transmission_color" => {
            if prop(m, "attenuationDistance")[0] > 0.0 {
                prop(m, "attenuationColor")
            } else {
                prop(m, "transmissionTint")
            }
        }
        "base_color" => base[..3].to_vec(),
        "geometry_opacity" => vec![base[3]],
        "transmission_dispersion_abbe_number" => {
            let d = prop(m, "dispersion")[0];
            vec![if d > 0.0 { 20.0 / d } else { 0.0 }]
        }
        "fuzz_weight" => vec![fuzz(m, last_fuzz).0],
        "fuzz_color" => fuzz(m, last_fuzz).1.to_vec(),
        "thin_film_thickness" => vec![prop(m, "iridescenceThicknessMaximum")[0] / 1000.0],
        "thin_film_ior" => prop(m, "iridescenceIor"),
        "emission_luminance" => prop(m, "emissiveStrength"),
        "emission_color" => prop(m, "emissiveFactor"),
        _ => return None,
    })
}

/// The stored properties to write for OpenPBR parameter `name` = `values`;
/// None if it isn't one, Err for the wrong number of values.
pub fn set(
    m: &Material,
    name: &str,
    values: &[f32],
    last_fuzz: Fuzz,
) -> Option<Result<Writes, String>> {
    let want = |n: usize| -> Result<(), String> {
        if values.len() == n {
            Ok(())
        } else {
            Err(format!(
                "\"{name}\" takes {n} value(s), got {}",
                values.len()
            ))
        }
    };
    if let Some((_, p)) = DIRECT.iter().find(|(n, _)| *n == name) {
        let len = prop(m, p).len();
        return Some(want(len).map(|()| vec![(*p, values.to_vec())]));
    }
    let base = prop(m, "baseColorFactor");
    let result = match name {
        "transmission_color" => want(3).map(|()| {
            if prop(m, "attenuationDistance")[0] > 0.0 {
                vec![("attenuationColor", values.to_vec())]
            } else {
                vec![("transmissionTint", values.to_vec())]
            }
        }),
        // The color moves with it: into the medium (absorbed) for a depth,
        // onto the surface (a tint) for none.
        "transmission_depth" => want(1).map(|()| {
            let depth = if values[0].is_finite() {
                values[0].max(0.0)
            } else {
                0.0
            };
            let had_depth = prop(m, "attenuationDistance")[0] > 0.0;
            let color = if had_depth {
                prop(m, "attenuationColor")
            } else {
                prop(m, "transmissionTint")
            };
            if depth > 0.0 {
                vec![
                    ("attenuationDistance", vec![depth]),
                    ("attenuationColor", color),
                    ("transmissionTint", vec![1.0; 3]),
                ]
            } else {
                vec![
                    ("attenuationDistance", vec![0.0]),
                    ("attenuationColor", vec![1.0; 3]),
                    ("transmissionTint", color),
                ]
            }
        }),
        "base_color" => want(3).map(|()| {
            vec![(
                "baseColorFactor",
                vec![values[0], values[1], values[2], base[3]],
            )]
        }),
        "geometry_opacity" => want(1).map(|()| {
            vec![(
                "baseColorFactor",
                vec![base[0], base[1], base[2], values[0]],
            )]
        }),
        "transmission_dispersion_abbe_number" => want(1).map(|()| {
            let d = if values[0] > 0.0 {
                20.0 / values[0]
            } else {
                0.0
            };
            vec![("dispersion", vec![d])]
        }),
        // The caller keeps the new pair as `last_fuzz` (`fuzz_after`).
        "fuzz_weight" => want(1).map(|()| {
            let (_, color) = fuzz(m, last_fuzz);
            vec![("sheenColorFactor", color.map(|c| c * values[0]).to_vec())]
        }),
        "fuzz_color" => want(3).map(|()| {
            let (w, _) = fuzz(m, last_fuzz);
            vec![("sheenColorFactor", values.iter().map(|c| c * w).collect())]
        }),
        "thin_film_thickness" => {
            want(1).map(|()| vec![("iridescenceThicknessMaximum", vec![values[0] * 1000.0])])
        }
        "thin_film_ior" => want(1).map(|()| vec![("iridescenceIor", values.to_vec())]),
        "emission_luminance" => want(1).map(|()| vec![("emissiveStrength", values.to_vec())]),
        "emission_color" => want(3).map(|()| vec![("emissiveFactor", values.to_vec())]),
        _ => return None,
    };
    Some(result)
}

/// The fuzz pair to keep after setting parameter `name` = `values`.
pub fn fuzz_after(m: &Material, name: &str, values: &[f32], last: Fuzz) -> Fuzz {
    let (w, color) = fuzz(m, last);
    match (name, values) {
        ("fuzz_weight", &[v]) => (v, color),
        ("fuzz_color", &[r, g, b]) => (w, [r, g, b]),
        _ => (w, color),
    }
}
