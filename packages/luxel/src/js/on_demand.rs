//! Render on demand: whether a `renderFrame` would draw anything new, so an
//! app (or the viewer) draws only then and shows the last frame again
//! (`presentFrame`) otherwise.
//!
//! What changes the picture is seen in four ways:
//! - the world's components changed since the last look (one O(1) check per
//!   component type: transforms, meshes, lights, cameras, visibility, …);
//! - a scene setter ran (each one calls `invalidate`);
//! - async work landed (`Context::content_version`: images, compression,
//!   environment bakes, reloads);
//! - the canvas changed size.
//!
//! What keeps changing it on its own is checked directly: animations
//! playing, shaders still compiling, an environment bake, model reloads,
//! auto exposure adapting, and TAA converging (several frames after a
//! change).

use luxel_ecs::{Entity, Tick};
use wasm_bindgen::prelude::*;

use super::Scene;

/// Frames TAA takes to settle after a change: its 8-frame jitter cycle a
/// few times over.
pub(super) const TAA_SETTLE: u32 = 30;

/// What `needsRender` keeps between frames.
#[derive(Default)]
pub(crate) struct OnDemand {
    // `invalidate` since the last look.
    invalidated: bool,
    // The world tick of the last look: later writes are changes.
    observed: Tick,
    // The context's content version at the last look.
    content: u32,
    // Frames still to draw after the last change (TAA converging).
    settle: u32,
    // What the last frame was drawn at, through, and with: the canvas size,
    // the camera, the context generation (a restore redraws).
    size: Option<(i32, i32)>,
    camera: Entity,
    generation: u32,
    // At the last look: the content version, and whether something kept
    // changing the picture on its own. Work that lands inside a frame (a
    // bake swapped in, a reload applied, the last shader linked) is no
    // change the next look sees, but TAA still needs frames to settle on it.
    looked_content: u32,
    looked_busy: bool,
}

#[wasm_bindgen]
impl Scene {
    /// Says the picture changed (something the scene can't see: a texture
    /// you filled yourself, a program's uniforms): `needsRender` is true
    /// until frames show it. Every scene setter does this on its own.
    pub fn invalidate(&mut self) {
        self.on_demand.invalidated = true;
    }

    /// Whether `renderFrame` would draw something new: a change since the
    /// last frame (the scene's setters, components, the canvas size, images
    /// or bakes landing — `invalidate` for the rest), or something still
    /// moving (animations playing, shaders compiling, an environment bake,
    /// auto exposure while it is on, TAA converging after a change). Pass
    /// the camera the next frame looks through: another one than the last
    /// frame's counts as a change. When false, `presentFrame` shows the last
    /// frame again (or nothing need be drawn).
    ///
    /// ```js
    /// scene.advanceAnimations(dt);
    /// if (scene.needsRender(camera) || !scene.presentFrame()) scene.renderFrame(camera);
    /// ```
    #[wasm_bindgen(js_name = needsRender)]
    pub fn needs_render(
        &mut self,
        #[wasm_bindgen(unchecked_param_type = "Entity | undefined")] camera: Option<u32>,
    ) -> bool {
        self.observe(camera.map(Entity::from_raw));
        // The context was restored but the scene not yet rebuilt (a frame
        // tried while lost used up the change).
        self.on_demand.settle > 0
            || self.keeps_changing()
            || self.generation != self.ctx.generation()
    }
}

impl Scene {
    // Folds what changed since the last look into `settle`.
    pub(crate) fn observe(&mut self, camera: Option<Entity>) {
        let content = self.ctx.content_version();
        let fb = self.ctx.default_framebuffer();
        let size = (fb.width(), fb.height());
        let generation = self.ctx.generation();
        let settle = if self.taa() { TAA_SETTLE } else { 1 };
        let busy = self.keeps_changing();
        let d = &mut self.on_demand;
        d.looked_content = content;
        d.looked_busy = busy;
        let changed = d.invalidated
            || self.world.changed_since(d.observed)
            || content != d.content
            || d.size != Some(size)
            || d.generation != generation
            || camera.is_some_and(|c| c != d.camera);
        if changed {
            d.invalidated = false;
            d.content = content;
            d.settle = settle;
        }
        // Writes from here on are newer.
        d.observed = self.world.advance_tick();
    }

    // After a frame: one fewer to settle; what the frame wrote itself (world
    // matrices, reload results) isn't a change.
    pub(crate) fn rendered(&mut self, camera: Entity) {
        let fb = self.ctx.default_framebuffer();
        let landed = self.ctx.content_version() != self.on_demand.looked_content
            || (self.on_demand.looked_busy && !self.keeps_changing());
        let taa = self.taa();
        let d = &mut self.on_demand;
        d.settle = d.settle.saturating_sub(1);
        if landed && taa {
            d.settle = d.settle.max(TAA_SETTLE - 1);
        }
        d.size = Some((fb.width(), fb.height()));
        d.camera = camera;
        d.generation = self.ctx.generation();
        d.content = self.ctx.content_version();
        d.observed = self.world.advance_tick();
    }

    // What changes the picture frame after frame on its own.
    fn keeps_changing(&self) -> bool {
        crate::animation::animating(&self.world)
            || self.stats.pending > 0
            || self.env_pending.is_some()
            || !self.pending_variants.is_empty()
            || !self.reloads.borrow().is_empty()
            // As drawn: without HDR output it does nothing.
            || self.auto_exposure()
    }
}
