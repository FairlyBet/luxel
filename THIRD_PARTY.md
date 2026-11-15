# Third-party code

Parts of the shaders in `packages/rswebgl-scene/shaders/` and of the glTF
loader (`packages/rswebgl-gltf/src/`) are ported from the projects below.
Each ported file says in its header what it took and from where. The originals' licenses apply to those parts.

| Project | License | Used in |
|---|---|---|
| [Khronos glTF-Sample-Renderer](https://github.com/KhronosGroup/glTF-Sample-Renderer) | Apache-2.0 | glTF reference model (`lib/surface_gltf.glsl`), GGX/Smith/Schlick (`lib/brdf.glsl`), transmission (`lib/surface.glsl`), sheen (`lib/sheen.glsl`), iridescence (`lib/iridescence.glsl`), the BRDF table's integration (`src/brdf_lut.rs`) |
| [MaterialX](https://github.com/AcademySoftwareFoundation/MaterialX) | Apache-2.0 | exact dielectric and F82 Fresnel, GGX directional-albedo fit, energy compensation (`lib/brdf.glsl`), EON diffuse (`lib/eon.glsl`), Zeltner fuzz (`lib/sheen.glsl`), Airy thin film (`lib/iridescence.glsl`), Burley-profile subsurface approximation (`lib/subsurface.glsl`), the OpenPBR layering it follows (`lib/surface_openpbr.glsl`) |
| [OpenPBR](https://github.com/AcademySoftwareFoundation/OpenPBR) | Apache-2.0 | the model `lib/surface_openpbr.glsl` implements |
| [Filament](https://github.com/google/filament) | Apache-2.0 | specular antialiasing and specular occlusion (`lib/brdf.glsl`); firefly suppression in the transmission copy's first mip (`grab_down.frag`, from `separableGaussianBlur.fs` `tapReinhard`); shadow maps (`lib/shadows.glsl`: the normal bias of `computeLightSpacePosition` and the 3×3 filter of `ShadowSample_PCF_Low`, after Castaño 2013; default biases from `ShadowOptions`) |
| [Bevy](https://github.com/bevyengine/bevy) | MIT / Apache-2.0 | the TAA resolve (`taa.frag`: Catmull-Rom history, YCoCg variance clip, confidence, reversible tone map; Halton jitter sequence), with Playdead's YCoCg helpers (MIT); shadow cascades (`lighting/views.rs` `cascade_views`: the stable fit of `calculate_cascade` after MJP's Shadows sample — the largest slice diagonal, the center moved in whole texels; `shadows.glsl`: the blend across the cascades' overlap of `sample_directional_cascade`; the split by Zhang et al. 2006, PSSM) |
| [Godot](https://github.com/godotengine/godot) | MIT | the 9-tap Catmull-Rom history filter in `taa.frag` (`taa_resolve.glsl`, Panos Karabelas' TAA, after MJP); auto exposure (`exposure.frag`, from Godot 3.5's GLES3 `exposure.glsl` and its shrink chain in `rasterizer_scene_gles3.cpp`) |
| [AMD FidelityFX FSR 1](https://github.com/GPUOpen-Effects/FidelityFX-FSR) | MIT | RCAS sharpening (`present.frag`, from `ffx_fsr1.h` `FsrRcasF`) |
| [three.js](https://github.com/mrdoob/three.js) | MIT | FXAA (`fxaa.frag`, from `FXAAShader`: NVIDIA's FXAA after Jasper Flick's implementation, GLSL by Dave Hoskins); ACES and AgX tone mapping (`lib/tonemap.glsl`); skinning and morph target vertex code (`pbr.vert`, after its `skinning_*` / `morphtarget_*` chunks); the decoder worker pools and KTX2 / Draco worker bodies (`rswebgl-gltf/src/decoders.js`, after `WorkerPool`, `KTX2Loader` and `DRACOLoader`), the KTX2 transcode-format choice; the WebP / AVIF support probes and their test images (`rswebgl-gltf/src/image_formats.rs`, r160 `GLTFTextureWebPExtension` / `GLTFTextureAVIFExtension`) |
| [meshoptimizer](https://github.com/zeux/meshoptimizer) | MIT | the `EXT_meshopt_compression` / `KHR_meshopt_compression` decoders (`rswebgl-gltf/src/meshopt.rs`, from `vertexcodec.cpp`, `indexcodec.cpp`, `vertexfilter.cpp`; its test vectors) |
| [Betsy](https://github.com/darksylinc/betsy) (Matias N. Goldberg, as in Godot's `modules/betsy`), with [stb_dxt](https://github.com/nothings/stb) (Fabian Giesen, Sean Barrett) | MIT (stb_dxt: MIT / public domain) | GPU texture compression: the BC1 and BC4 encoders in `rswebgl/src/compress/bc.frag` (`bc1.glsl`, `bc4.glsl`), the ETC1 encoder in `compress/etc.frag` (`etc1.glsl`, after rg-etc1 by Rich Geldreich, zlib) and stb_dxt's endpoint tables (`compress/tables.rs`, from `betsy_bc1.h`) |
| [etcpak](https://github.com/wolfpld/etcpak) (Bartosz Taudul) | BSD-3-Clause | the EAC encoder in `rswebgl/src/compress/etc.frag` (`ProcessRGB.cpp`, `ProcessAlpha_ETC2`) |
| [astc_encoder](https://github.com/niepp/astc_encoder) (niepp), with [ARM astc-encoder](https://github.com/ARM-software/astc-encoder)'s tables | MIT (astc-encoder: Apache-2.0) | the ASTC 4×4 encoder in `rswebgl/src/compress/astc.frag` (`ASTC_Encode.hlsl`, `ASTC_IntegerSequenceEncoding.hlsl`, `ASTC_Table.hlsl`) |
| [GPURealTimeBC6H](https://github.com/knarkowicz/GPURealTimeBC6H) (Krzysztof Narkowicz), as in Godot's `modules/betsy/bc6h.glsl` | public domain (Godot's port: MIT) | the BC6H encoder in `rswebgl/src/compress/bc6h.frag` |

Apache License 2.0: https://www.apache.org/licenses/LICENSE-2.0.
Changes from the originals: renamed and restructured into this project's
chunks; per-file notes describe behavioural differences.
