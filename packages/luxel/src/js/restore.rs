//! Rebuilding the scene after a context loss.
//!
//! A loss kills every GPU object; parameter groups (materials) and the world
//! are CPU-side and survive. The scene notices the new context generation at
//! its next call (a frame, or anything that creates GPU objects) and rebuilds
//! what it made itself, keeping every handle the app holds that can be kept:
//!
//! - its renderer (frame and lighting blocks, default environment, BRDF
//!   table, background shader), with exposure / intensity / rotation;
//! - the environment, baked again from the `.hdr` bytes it was given;
//! - PBR variants and `createProgram` shaders, recompiled, meshes switched;
//! - `loadTexture` textures, loaded again; materials repointed;
//! - models, fetched again from their URL (`Model::adopt`): until a reload
//!   lands its meshes are left out of frames (`Reloading`); material handles
//!   and edits stay as they were.
//!
//! What the app made itself (programs from `ctx`, meshes from `setMesh`,
//! textures not from `loadTexture`) is the app's to rebuild, in
//! `ctx.setOnRestored`.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use luxel_ecs::Entity;
use luxel_gl::console;
use luxel_gl::error::Error;
use luxel_gl::ids::{ProgramId, SamplerId, TextureId};
use luxel_gltf::Model;
use wasm_bindgen::prelude::*;

use super::Scene;

use crate::hdr::decode_hdr_rgbe;
use crate::model::{Morph, Primitive};
use crate::pbr::PbrShaders;
use crate::render::{MeshRenderer, Reloading};
use crate::shader::create_scene_program;

/// Models reloaded after a loss, waiting for the scene's next frame: the
/// model root and the fresh load.
pub(crate) type Reloads = Rc<RefCell<Vec<(Entity, Result<Model, Error>)>>>;

type LossListener = Closure<dyn FnMut(web_sys::Event)>;

/// The app's `setOnLost` / `setOnRestored` callbacks: the first called
/// from a listener on the canvas (the context's own slot stays the app's),
/// the second once the scene has rebuilt.
#[derive(Default)]
pub(crate) struct LossEvents {
    on_lost: Rc<RefCell<Option<js_sys::Function>>>,
    on_restored: Option<js_sys::Function>,
    listener: Option<(web_sys::EventTarget, LossListener)>,
}

impl Drop for LossEvents {
    fn drop(&mut self) {
        if let Some((target, f)) = self.listener.take() {
            let _ = target.remove_event_listener_with_callback(
                "webglcontextlost",
                f.as_ref().unchecked_ref(),
            );
        }
    }
}

/// A scene shader's sources, to compile it again after a loss.
pub(crate) struct SceneProgram {
    pub(crate) id: ProgramId,
    pub(crate) vert: String,
    pub(crate) frag: String,
}

impl Scene {
    /// Rebuilds after a context loss if one happened since the scene last
    /// looked; false while the context is lost (nothing can be made then).
    pub(crate) fn sync_context(&mut self) -> bool {
        if self.ctx.is_lost() {
            return false;
        }
        if self.generation != self.ctx.generation() {
            self.restore();
        }
        true
    }

    fn restore(&mut self) {
        let ctx = self.ctx.clone();
        self.generation = ctx.generation();
        // What an earlier restore started is moot now.
        self.restore_tasks.clear();
        console::log("[luxel] rebuilding the scene after a context loss");

        if let Err(e) = self.renderer.rebuild(&ctx) {
            console::error(&format!("[luxel] restore: the renderer: {e}"));
        }
        // The bake shaders went with the old context; a bake in progress too.
        let intensity = self.intensity_to_keep();
        if let Some(p) = self.env_pending.take() {
            p.cancel();
        }
        if let Some(baker) = self.env_baker.take() {
            baker.destroy();
        }
        if let Some((bytes, size)) = self.env_source.clone() {
            let started = decode_hdr_rgbe(&bytes)
                .map_err(Error::from)
                .and_then(|img| self.start_environment(&img, size));
            match started {
                // Baked over the next frames; the intensity survives its swap.
                Ok(()) => self.env_intensity_after_bake = intensity,
                Err(e) => console::error(&format!("[luxel] restore: the environment: {e}")),
            }
        }
        self.restore_programs();
        self.restore_textures();
        self.reload_models();
        // After this call returns (the app may call the scene from it).
        if let Some(f) = self.loss_events.on_restored.clone() {
            let alive = self.alive.clone();
            wasm_bindgen_futures::spawn_local(async move {
                // Not for a scene destroyed since.
                if !alive.get() {
                    return;
                }
                if let Err(e) = f.call0(&JsValue::NULL) {
                    console::error(&format!("[luxel] onRestored threw: {e:?}"));
                }
            });
        }
    }

    // Recompiles PBR variants and scene shaders and switches meshes to them.
    fn restore_programs(&mut self) {
        let old_pbr = std::mem::replace(&mut self.pbr, PbrShaders::new());
        self.pbr.set_model(old_pbr.model());
        self.pending_variants.clear();
        let mut remap: HashMap<ProgramId, ProgramId> = HashMap::new();
        // Every one, used or not: the app may hold a handle for later
        // (`currentProgram` maps it).
        self.scene_programs.retain_mut(|p| {
            match create_scene_program(&self.ctx, &p.vert, &p.frag) {
                Ok(new) => {
                    remap.insert(p.id, new);
                    p.id = new;
                    true
                }
                Err(e) => {
                    console::error(&format!("[luxel] restore: a scene shader: {e}"));
                    false
                }
            }
        });
        // Handles from before (this loss or earlier ones) lead to the new.
        for target in self.program_aliases.values_mut() {
            if let Some(&new) = remap.get(target) {
                *target = new;
            }
        }
        self.program_aliases
            .extend(remap.iter().map(|(&old, &new)| (old, new)));
        let meshes: Vec<(Entity, ProgramId)> = self
            .world
            .storage::<MeshRenderer>()
            .map(|m| m.iter().map(|(e, m)| (e, m.program)).collect())
            .unwrap_or_default();
        for (e, program) in meshes {
            let new = if old_pbr.is_pbr(program) {
                self.pbr_program(e)
            } else {
                remap.get(&program).copied()
            };
            if let (Some(new), Some(mut m)) = (new, self.world.get_mut::<MeshRenderer>(e)) {
                m.program = new;
            }
            self.refresh_shadow_caster(e);
        }
    }

    // The PBR variant for primitive entity `e` with its current material.
    fn pbr_program(&mut self, e: Entity) -> Option<ProgramId> {
        let mesh = self.world.get::<MeshRenderer>(e)?;
        let attributes = self.world.get::<Primitive>(e).map_or(0, |p| p.attributes);
        let material = self
            .materials
            .get(&mesh.material)
            .map(|r| r.material.clone());
        let material = material.unwrap_or_else(luxel_gltf::default_material);
        let key = crate::pbr::PbrKey::new(attributes, &material);
        self.pbr
            .program(&self.ctx, key)
            .map_err(|e| console::error(&format!("[luxel] restore: a PBR variant: {e}")))
            .ok()
    }

    // Loads `loadTexture`'s textures again and points materials at them.
    fn restore_textures(&mut self) {
        let mut remap = HashMap::new();
        for t in &mut self.textures {
            let url = t.spec.url().to_string();
            // Settled once it loads (or fails): `whenReady` waits for it.
            let mut resolve = None;
            let loaded = js_sys::Promise::new(&mut |r, _| resolve = Some(r));
            let resolve = resolve.expect("the executor runs at once");
            let ctx = self.ctx.clone();
            let on_load = move |tex: TextureId| -> js_sys::Function {
                Closure::once_into_js(move |w: f64, _h: f64| {
                    // Not for a texture gone meanwhile (another loss).
                    if w == 0.0 && ctx.texture_is_valid(tex) {
                        console::error(&format!("[luxel] restore: can't load {url}"));
                    }
                    let _ = resolve.call0(&JsValue::NULL);
                })
                .unchecked_into()
            };
            match t.spec.start(&self.ctx, on_load) {
                Ok(new) => {
                    remap.insert(t.id, new);
                    t.id = new;
                    self.restore_tasks.push(loaded);
                }
                Err(e) => console::error(&format!("[luxel] restore: a texture: {e}")),
            }
        }
        self.remap_textures(&remap, &HashMap::new());
        // The app's handles from before (this loss or earlier ones) lead to
        // the new textures.
        for target in self.texture_aliases.values_mut() {
            if let Some(&new) = remap.get(target) {
                *target = new;
            }
        }
        self.texture_aliases
            .extend(remap.iter().map(|(&old, &new)| (old, new)));
    }

    // A `loadTexture` handle as it is now (the reloaded one after a loss).
    pub(crate) fn current_texture(&self, t: TextureId) -> TextureId {
        self.texture_aliases.get(&t).copied().unwrap_or(t)
    }

    // Points every material (groups and the scene's records) at new textures.
    fn remap_textures(
        &mut self,
        textures: &HashMap<TextureId, TextureId>,
        samplers: &HashMap<SamplerId, SamplerId>,
    ) {
        self.ctx.remap_group_handles(textures, samplers);
        for record in self.materials.values_mut() {
            for t in &mut record.maps {
                if let Some(&new) = textures.get(t) {
                    *t = new;
                }
            }
        }
    }

    // The model at `root` (an instance's too).
    pub(crate) fn model(&self, root: Entity) -> Option<&Model> {
        self.models.get(self.model_roots.get(&root)?)
    }

    // Every root model `key` is spawned under.
    fn roots_of(&self, key: Entity) -> Vec<Entity> {
        self.model_roots
            .iter()
            .filter(|&(_, &k)| k == key)
            .map(|(&r, _)| r)
            .collect()
    }

    // Starts fetching every model again; its meshes wait meanwhile.
    fn reload_models(&mut self) {
        let roots: Vec<Entity> = self.models.keys().copied().collect();
        for root in roots {
            self.reload_model(root);
        }
    }

    // Starts fetching `root`'s model again if its GPU objects are from a
    // lost context.
    pub(crate) fn reload_model(&mut self, root: Entity) {
        {
            let Some(model) = self.models.get(&root) else {
                return;
            };
            if !model.is_lost(&self.ctx) {
                return;
            }
            let request = model.reload_request().filter(|_| self.restore_models);
            let Some(request) = request else {
                // The app reloads it (setRestoreModels(false)).
                console::warn_once(
                    "[luxel] restore: models aren't fetched again \
                     (setRestoreModels(false)): their meshes stay out of frames until replaced",
                );
                self.mark_reloading(root);
                return;
            };
            self.mark_reloading(root);
            let (ctx, inbox, alive) = (self.ctx.clone(), self.reloads.clone(), self.alive.clone());
            let in_flight = self.reloads_in_flight.clone();
            in_flight.set(in_flight.get() + 1);
            // Settles once the model is back and its images are in, or
            // rejects with why it couldn't be: `whenReady` waits for it and
            // passes a failure on (the app can despawn it and add it again).
            let task = wasm_bindgen_futures::future_to_promise(async move {
                let result = luxel_gltf::load_request(&ctx, request).await;
                in_flight.set(in_flight.get().saturating_sub(1));
                if !alive.get() {
                    // The scene was destroyed meanwhile.
                    if let Ok(model) = result {
                        model.destroy(&ctx);
                    }
                    return Ok(JsValue::UNDEFINED);
                }
                let images = result.as_ref().ok().map(|m| m.when_loaded());
                let failed = result.as_ref().err().map(|e| e.to_string());
                inbox.borrow_mut().push((root, result));
                if let Some(images) = images {
                    let _ = wasm_bindgen_futures::JsFuture::from(images).await;
                }
                match failed {
                    Some(e) => {
                        Err(Error::from(format!("a model couldn't be reloaded: {e}")).into())
                    }
                    None => Ok(JsValue::UNDEFINED),
                }
            });
            self.restore_tasks.push(task);
        }
    }

    // The model's own meshes (its primitives: not the app's meshes parented
    // under its nodes) leave frames until the reload lands.
    fn mark_reloading(&mut self, root: Entity) {
        for r in self.roots_of(root) {
            for e in self.own_entities(r) {
                if self.world.has::<MeshRenderer>(e) && self.world.has::<Primitive>(e) {
                    self.world.insert(e, Reloading);
                }
            }
        }
    }

    /// Puts reloaded models in place (called each frame).
    pub(crate) fn apply_reloads(&mut self) {
        if self.reloads.borrow().is_empty() {
            return;
        }
        let done = std::mem::take(&mut *self.reloads.borrow_mut());
        self.ctx.note_content_changed();
        for (root, result) in done {
            let fresh = match result {
                Ok(fresh) => fresh,
                Err(e) => {
                    // A second loss mid-reload starts another reload; anything
                    // else leaves the model out, reported.
                    if !self.ctx.is_lost() && self.models.contains_key(&root) {
                        console::error(&format!("[luxel] restore: a model: {e}"));
                    }
                    continue;
                }
            };
            let Some(model) = self.models.get_mut(&root) else {
                fresh.destroy(&self.ctx); // despawned meanwhile
                continue;
            };
            if fresh.is_lost(&self.ctx) || !model.is_lost(&self.ctx) {
                fresh.destroy(&self.ctx); // from a context lost since, or stale
                continue;
            }
            let old: Vec<TextureId> = (0..model.texture_count())
                .map(|i| model.texture(i))
                .collect();
            if let Err(e) = model.adopt(&self.ctx, fresh) {
                console::error(&format!("[luxel] restore: {e}"));
                continue;
            }
            let textures: HashMap<TextureId, TextureId> = old
                .into_iter()
                .zip((0..model.texture_count()).map(|i| model.texture(i)))
                .collect();
            // `adopt` repointed the groups; the scene's records follow.
            for record in self.materials.values_mut() {
                for t in &mut record.maps {
                    if let Some(&new) = textures.get(t) {
                        *t = new;
                    }
                }
            }
            let items: Vec<_> = model.items().iter().map(|i| (i.vao(), i.morph())).collect();
            // Its own meshes (a model hung under one of its roots reloads
            // on its own).
            let meshes: Vec<Entity> = self
                .roots_of(root)
                .into_iter()
                .flat_map(|r| self.own_entities(r))
                .collect();
            for e in meshes {
                let Some(item) = self.world.get::<Primitive>(e).map(|p| p.item as usize) else {
                    continue;
                };
                let Some(&(own, morph)) = items.get(item) else {
                    continue;
                };
                // The new vertex array its material needs, at once (the old
                // is dead); render states outlive a loss, so its own stays.
                let (vao, attributes) = self.wanted_vertex_array(e).unwrap_or_else(|| {
                    (
                        own,
                        self.world.get::<Primitive>(e).map_or(0, |p| p.attributes),
                    )
                });
                Self::set_vertex_array(&mut self.world, e, vao, attributes);
                if let (Some((texture, _)), Some(mut m)) = (morph, self.world.get_mut::<Morph>(e)) {
                    m.texture = texture;
                }
                self.world.remove::<Reloading>(e);
                // Its shader variant for that layout (and its shadow caster).
                self.apply_material(e);
            }
        }
    }
}

#[wasm_bindgen]
impl Scene {
    /// Calls `f` when the WebGL context is lost (frames draw nothing until
    /// it is back); undefined removes it. Beside `ctx.setOnLost`, which
    /// stays the app's.
    #[wasm_bindgen(js_name = setOnLost)]
    pub fn set_on_lost(
        &mut self,
        #[wasm_bindgen(unchecked_param_type = "(() => void) | undefined")] f: Option<
            js_sys::Function,
        >,
    ) {
        *self.loss_events.on_lost.borrow_mut() = f;
        if self.loss_events.listener.is_some() {
            return;
        }
        let Some(target) = self
            .ctx
            .raw_gl()
            .canvas()
            .and_then(|c| c.dyn_into::<web_sys::EventTarget>().ok())
        else {
            return;
        };
        let slot = self.loss_events.on_lost.clone();
        let listener = Closure::<dyn FnMut(web_sys::Event)>::new(move |_| {
            let f = slot.borrow().clone();
            if let Some(f) = f
                && let Err(e) = f.call0(&JsValue::NULL)
            {
                console::error(&format!("[luxel] onLost threw: {e:?}"));
            }
        });
        let _ = target.add_event_listener_with_callback(
            "webglcontextlost",
            listener.as_ref().unchecked_ref(),
        );
        self.loss_events.listener = Some((target, listener));
    }

    /// Calls `f` once the scene has rebuilt itself after a context loss —
    /// at its first call after the context is back (a frame, `needsRender`)
    /// — to rebuild what the app made itself (`setMesh` meshes, programs
    /// from `ctx`). Models are being fetched again then: `whenReady`
    /// waits for them. Undefined removes it.
    #[wasm_bindgen(js_name = setOnRestored)]
    pub fn set_on_restored(
        &mut self,
        #[wasm_bindgen(unchecked_param_type = "(() => void) | undefined")] f: Option<
            js_sys::Function,
        >,
    ) {
        self.loss_events.on_restored = f;
    }

    /// The live handle of a `createProgram` shader: after a context loss the
    /// scene compiles it again under a new handle, and this maps the one you
    /// hold to it (the same handle when there was no loss). `setMesh` and
    /// `addModelWithProgram` accept either.
    #[wasm_bindgen(js_name = currentProgram)]
    pub fn current_program(&self, program: ProgramId) -> ProgramId {
        self.program_aliases
            .get(&program)
            .copied()
            .unwrap_or(program)
    }

    /// Whether models are fetched again from their URL after a context loss
    /// (default true). Off, the scene keeps no model sources (a `data:` URI
    /// can be a whole file): after a loss their meshes stay out of frames and
    /// the app loads them again itself (`ctx.setOnRestored`).
    #[wasm_bindgen(js_name = setRestoreModels)]
    pub fn set_restore_models(&mut self, on: bool) {
        self.restore_models = on;
        if !on {
            for model in self.models.values_mut() {
                model.forget_source();
            }
        }
    }
}

impl Scene {
    // Keeps a model added to the scene (its source too, unless
    // `setRestoreModels(false)`).
    pub(crate) fn keep_model(&mut self, root: Entity, mut model: Model) {
        if !self.restore_models {
            model.forget_source();
        }
        self.models.insert(root, model);
        self.model_roots.insert(root, root);
    }
}
