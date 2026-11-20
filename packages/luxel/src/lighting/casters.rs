//! Which casters each shadow map draws: every visible `ShadowCaster` the
//! light's camera sees — culled or not by the scene camera's (a mesh out of
//! view still shadows what is in it). The renderer gives each its object
//! (matrices, instance, deformers) and pushes it with `push`.

use luxel_ecs::{Entity, World};
use luxel_gl::context::Context;
use luxel_gl::ids::ProgramId;

use super::settings::ShadowCaster;
use super::views::ShadowView;
use crate::culling::{Bounds, Frustum};
use crate::render::{MeshRenderer, Reloading, hidden};
use crate::transform::WorldMatrix;

/// A caster to draw into shadow map `view`.
#[derive(Debug, Clone, Copy)]
pub(crate) struct CasterDraw {
    pub view: usize,
    pub entity: Entity,
    pub program: ProgramId,
    pub mesh: MeshRenderer,
    /// Its row in the `WorldMatrix` storage.
    pub row: u32,
}

/// This frame's caster draws for `views`, into `out` (cleared first).
/// Blended meshes cast none; neither do those whose shader still links or
/// whose model reloads: returns how many such waits there were.
pub(crate) fn cull(
    world: &World,
    ctx: &Context,
    views: &[ShadowView],
    out: &mut Vec<CasterDraw>,
) -> usize {
    out.clear();
    let (Some(casters), Some(meshes), Some(worlds)) = (
        world.storage::<ShadowCaster>(),
        world.storage::<MeshRenderer>(),
        world.storage::<WorldMatrix>(),
    ) else {
        return 0;
    };
    if views.is_empty() {
        return 0;
    }
    let mut waiting = 0;
    let bounds = world.storage::<Bounds>();
    let reloading = world.storage::<Reloading>().filter(|r| !r.is_empty());
    let any_hidden = crate::render::any_hidden(world);
    // The checks that don't depend on the view, once per caster (and a
    // waiting caster counted once): what passes, with its world box.
    let mut ready = Vec::new();
    for (e, caster) in casters.iter() {
        let Some(mesh) = meshes.get(e) else {
            continue;
        };
        if mesh.transparent || caster.program.is_null() || (any_hidden && hidden(world, e)) {
            continue;
        }
        // A reloading mesh is out of the maps until it lands (its reload
        // changes the content version, which redraws them).
        if reloading.is_some_and(|r| r.contains(e)) {
            continue;
        }
        if ctx.program_link_pending(caster.program) {
            waiting += 1;
            continue;
        }
        let Some(row) = worlds.row(e) else {
            continue;
        };
        let world_box = bounds
            .and_then(|b| b.get(e))
            .map(|b| b.transformed(&worlds.data()[row as usize].0));
        ready.push((e, caster.program, *mesh, row, world_box));
    }
    // Grouped by view, as the map passes take them.
    let reverse = ctx.reverse_z();
    for (i, view) in views.iter().enumerate() {
        let frustum = Frustum::from_view_proj_depth(&view.view_proj, reverse);
        for &(entity, program, mesh, row, world_box) in &ready {
            if world_box.is_some_and(|(c, h)| !frustum.intersects(c, h)) {
                continue;
            }
            out.push(CasterDraw {
                view: i,
                entity,
                program,
                mesh,
                row,
            });
        }
    }
    waiting
}
