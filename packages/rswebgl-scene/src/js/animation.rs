//! A model's glTF animations for JS: list, play, stop, advance.

use rswebgl::error::Error;
use rswebgl_ecs::Entity;
use wasm_bindgen::prelude::*;

use super::{Scene, options};
use crate::animation::{
    AnimationEvent, Fade, ModelAnimations, Playback, advance, apply, apply_fade, fade_base, place,
};

#[wasm_bindgen(typescript_custom_section)]
const TS_ANIMATION: &str = r#"
/** What an animation did in an `advanceAnimations`. */
export interface AnimationEvent {
  /** "finished": a clip that doesn't loop reached its end (its start,
   *  played backwards); "loop": a looping clip wrapped round. */
  type: "finished" | "loop";
  /** The model's root. */
  root: Entity;
  /** The animation's index. */
  animation: number;
}
"#;

#[wasm_bindgen]
impl Scene {
    /// The names of the animations of the model at `root` (`addModel`'s
    /// entity), in file order; "Animation N" for an unnamed one.
    pub fn animations(&self, root: Entity) -> Vec<String> {
        self.world
            .get::<ModelAnimations>(root)
            .map(|a| {
                a.clips
                    .iter()
                    .enumerate()
                    .map(|(i, c)| {
                        c.name
                            .clone()
                            .unwrap_or_else(|| format!("Animation {}", i + 1))
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Seconds animation `index` of `root`'s model lasts (0 if there is none).
    #[wasm_bindgen(js_name = animationDuration)]
    pub fn animation_duration(&self, root: Entity, index: usize) -> f32 {
        self.world
            .get::<ModelAnimations>(root)
            .and_then(|a| a.clips.get(index).map(|c| c.duration))
            .unwrap_or(0.0)
    }

    /// Plays animation `index` of `root`'s model from its start, posing the
    /// model at once; `advanceAnimations` moves it on. Looping wraps at the
    /// end; otherwise it holds the last pose. Replaces what that model played.
    #[wasm_bindgen(js_name = playAnimation)]
    pub fn play_animation(
        &mut self,
        root: Entity,
        index: usize,
        looping: bool,
    ) -> Result<(), Error> {
        self.invalidate();
        let Some(anims) = self
            .world
            .get::<ModelAnimations>(root)
            .map(|a| (*a).clone())
        else {
            return Err("playAnimation: no animations on that entity (a model root?)".into());
        };
        let Some(clip) = anims.clips.get(index) else {
            return Err(format!("playAnimation: no animation {index}").into());
        };
        apply(&mut self.world, clip, &anims.nodes, 0.0);
        if let Some(mut a) = self.world.get_mut::<ModelAnimations>(root) {
            a.playing = Some(Playback {
                clip: index,
                time: 0.0,
                speed: 1.0,
                looping,
            });
            a.fade = None;
        }
        Ok(())
    }

    /// Plays animation `index` from its start, fading into it over
    /// `seconds` from what played: the two poses mixed, the new one's
    /// weight rising from 0 to 1, both clips moving on meanwhile (the old
    /// at its speed; the new at 1). A property only one of them animates
    /// mixes with where it stood when the fade began. Fading again during a
    /// fade starts from the pose then. Without a playing animation, or with
    /// `seconds` ≤ 0, as `playAnimation`.
    #[wasm_bindgen(js_name = crossfadeAnimation)]
    pub fn crossfade_animation(
        &mut self,
        root: Entity,
        index: usize,
        seconds: f32,
        looping: bool,
    ) -> Result<(), Error> {
        let Some(anims) = self
            .world
            .get::<ModelAnimations>(root)
            .map(|a| (*a).clone())
        else {
            return Err("crossfadeAnimation: no animations on that entity (a model root?)".into());
        };
        if anims.clips.get(index).is_none() {
            return Err(format!("crossfadeAnimation: no animation {index}").into());
        }
        let Some(from) = anims.playing.filter(|_| seconds > 0.0 && seconds.is_finite()) else {
            return self.play_animation(root, index, looping);
        };
        self.invalidate();
        let base = fade_base(&self.world, &anims.clips, &anims.nodes, &from, index);
        let to = Playback {
            clip: index,
            time: 0.0,
            speed: 1.0,
            looping,
        };
        let fade = Fade {
            from,
            elapsed: 0.0,
            duration: seconds,
            base: base.into(),
        };
        apply_fade(&mut self.world, &anims.clips, &anims.nodes, &to, &fade);
        if let Some(mut a) = self.world.get_mut::<ModelAnimations>(root) {
            a.playing = Some(to);
            a.fade = Some(fade);
        }
        Ok(())
    }

    /// Moves `root`'s animation to `seconds` into its clip (wrapped if it
    /// loops, else held within it) and poses the model there; a crossfade
    /// under way ends. Nothing without a playing animation.
    #[wasm_bindgen(js_name = setAnimationTime)]
    pub fn set_animation_time(&mut self, root: Entity, seconds: f32) {
        self.invalidate();
        if !options::finite("setAnimationTime", &[seconds]) {
            return;
        }
        let Some(anims) = self
            .world
            .get::<ModelAnimations>(root)
            .map(|a| (*a).clone())
        else {
            return;
        };
        let Some(mut p) = anims.playing else {
            return;
        };
        let Some(clip) = anims.clips.get(p.clip) else {
            return;
        };
        p.time = place(seconds, clip.duration, p.looping);
        apply(&mut self.world, clip, &anims.nodes, p.time);
        if let Some(mut a) = self.world.get_mut::<ModelAnimations>(root) {
            a.playing = Some(p);
            a.fade = None;
        }
    }

    /// Stops `root`'s animation where it is (the pose stays).
    #[wasm_bindgen(js_name = stopAnimation)]
    pub fn stop_animation(&mut self, root: Entity) {
        self.invalidate();
        if let Some(mut a) = self.world.get_mut::<ModelAnimations>(root) {
            a.playing = None;
            a.fade = None;
        }
    }

    /// Seconds of animation per second of `advanceAnimations` (default 1;
    /// negative plays backwards; a non-finite speed counts as 1).
    #[wasm_bindgen(js_name = setAnimationSpeed)]
    pub fn set_animation_speed(&mut self, root: Entity, speed: f32) {
        self.invalidate();
        if !options::finite("setAnimationSpeed", &[speed]) {
            return;
        }
        if let Some(mut a) = self.world.get_mut::<ModelAnimations>(root)
            && let Some(p) = a.playing.as_mut()
        {
            p.speed = if speed.is_finite() { speed } else { 1.0 };
        }
    }

    /// Where `root`'s animation is, in seconds; undefined when none plays.
    #[wasm_bindgen(js_name = animationTime)]
    pub fn animation_time(&self, root: Entity) -> Option<f32> {
        self.world
            .get::<ModelAnimations>(root)
            .and_then(|a| a.playing.map(|p| p.time))
    }

    /// Moves every playing animation (and crossfade) on by `seconds` and
    /// poses its model — call once a frame with the time since the last,
    /// before `renderFrame`. Returns what clips did on the way: one that
    /// finished, one that looped (at most one event per clip a call).
    #[wasm_bindgen(js_name = advanceAnimations, unchecked_return_type = "AnimationEvent[]")]
    pub fn advance_animations(&mut self, seconds: f32) -> js_sys::Array {
        let _span = tracing::info_span!("scene.advanceAnimations").entered();
        let out = js_sys::Array::new();
        for event in advance(&mut self.world, seconds) {
            let (kind, root, clip) = match event {
                AnimationEvent::Finished { root, clip } => ("finished", root, clip),
                AnimationEvent::Looped { root, clip } => ("loop", root, clip),
            };
            let o = js_sys::Object::new();
            let _ = js_sys::Reflect::set(&o, &"type".into(), &kind.into());
            let _ = js_sys::Reflect::set(&o, &"root".into(), &root.to_raw().into());
            let _ = js_sys::Reflect::set(&o, &"animation".into(), &(clip as u32).into());
            out.push(&o);
        }
        out
    }
}

#[wasm_bindgen]
impl Scene {
    /// The morph target weights of the mesh at node `e` (a glTF node entity:
    /// `findByName`); empty without morph targets.
    #[wasm_bindgen(js_name = morphWeights)]
    pub fn morph_weights(&self, e: Entity) -> Vec<f32> {
        self.world
            .get::<crate::animation::MorphWeights>(e)
            .map(|w| w.0.clone())
            .unwrap_or_default()
    }

    /// Sets the morph target weights of the mesh at node `e` (an animation
    /// playing on its model overwrites them).
    #[wasm_bindgen(js_name = setMorphWeights)]
    pub fn set_morph_weights(&mut self, e: Entity, weights: Vec<f32>) {
        self.invalidate();
        if !options::finite("setMorphWeights", &weights) {
            return;
        }
        if self.world.is_alive(e) {
            self.world
                .insert(e, crate::animation::MorphWeights(weights));
        }
    }
}
