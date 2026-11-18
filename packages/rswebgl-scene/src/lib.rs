//! Scene components and systems on top of `rswebgl-ecs` (design:
//! `docs/design/ecs.md`).
//!
//! - [`Transform`]: an entity's local position, rotation and scale.
//! - [`Parent`]: makes the entity's transform relative to another entity's.
//! - [`WorldMatrix`]: the resulting world matrix, written by
//!   [`TransformSystem`] — never by hand.
//! - [`Camera`], [`Bounds`], [`MeshRenderer`], [`Visible`]: what
//!   [`SceneRenderer`] needs to cull the scene and push its draws into a
//!   `Pass`.
//!
//! ```
//! use glam::Vec3;
//! use rswebgl_ecs::World;
//! use rswebgl_scene::{Transform, TransformSystem, WorldMatrix, set_parent};
//!
//! let mut world = World::new();
//! let ring = world.spawn();
//! world.insert(ring, Transform::from_translation(Vec3::new(1.0, 0.0, 0.0)));
//! let stone = world.spawn();
//! world.insert(stone, Transform::from_translation(Vec3::new(0.0, 2.0, 0.0)));
//! set_parent(&mut world, stone, Some(ring)).unwrap();
//!
//! let mut transforms = TransformSystem::new();
//! transforms.update(&mut world);
//! let m = world.get::<WorldMatrix>(stone).unwrap().0;
//! assert_eq!(m.w_axis.truncate(), Vec3::new(1.0, 2.0, 0.0));
//! world.trim_removed(transforms.last_run());
//! ```

mod animation;
mod brdf_lut;
mod camera;
mod culling;
mod direct;
mod environment;
mod exposure;
mod framing;
mod fullscreen;
mod gpu_pick;
mod hdr;
mod hierarchy;
mod instances;
mod js;
mod lighting;
mod model;
mod motion;
mod occlusion;
mod output;
mod pbr;
mod pick;
mod render;
mod shader;
mod transform;

pub use animation::{AnimationEvent, Fade, ModelAnimations, MorphWeights, Playback};
pub use camera::{Camera, Projection};
pub use culling::{Bounds, Frustum};
pub use environment::{BakeJob, EnvBaker, Environment, PendingEnvironment};
pub use framing::{frame_box, world_bounds};
pub use hdr::{HdrImage, RgbeImage, decode_hdr, decode_hdr_rgbe, encode_hdr};
pub use hierarchy::{
    HierarchyError, TransformSystem, children, descendants, despawn_recursive, set_parent,
};
pub use js::Scene;
pub use lighting::{
    GroundShadow, LIGHTS_BLOCK, Light, LightKind, LightShadow, MAX_CASCADES, MAX_LIGHTS,
    MAX_SHADOW_LAYERS, SHADOW_MAP, ShadowCaster,
};
pub use model::{Morph, Name, Primitive, Skin, find_by_name, spawn_model};
pub use occlusion::{AmbientOcclusion, ContactShadows};
pub use output::{HdrOutput, ToneMapping};
pub use pbr::{PbrKey, PbrShaders, ShadingModel};
pub use pick::{Hit, Ray, camera_ray, pick_bounds, ray_aabb, view_and_projection, world_to_ndc};
pub use render::{
    FRAME_BLOCK, FRAME_SLOT, INSTANCE_BASE_UNIFORM, INSTANCES_TEXTURE, JOINT_BASE_UNIFORM,
    LIGHTING_BLOCK, MATERIAL_SLOT, MODEL_UNIFORM, MORPH_BASE_UNIFORM, MeshRenderer,
    NORMAL_MATRIX_UNIFORM, OBJECT_ID_UNIFORM, OBJECT_SLOT, Reloading, RenderStats, SceneRenderer,
    Visible,
};
pub use shader::{CHUNKS, FRAG_PRELUDE, VERT_PRELUDE, chunk, create_scene_program, shader_sources};
pub use transform::{Parent, Transform, WorldMatrix};
