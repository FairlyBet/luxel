//! Playing glTF animations: a channel sampled at a time (glTF 2.0 Appendix C
//! — step; linear, spherical for rotations; cubic Hermite splines) and the
//! result written into the node entities' `Transform`s and `MorphWeights`.

use std::collections::HashMap;
use std::rc::Rc;

use glam::{Quat, Vec3};
use rswebgl_ecs::{Entity, World};
use rswebgl_gltf::{Animation, AnimationChannel, AnimationPath, Interpolation};

use crate::transform::Transform;

/// Morph target weights of the mesh on this (node) entity; its primitives
/// read them. Animations write them.
#[derive(Debug, Clone, PartialEq)]
pub struct MorphWeights(pub Vec<f32>);

/// A model's animations, on its root entity: the clips, which entity each
/// glTF node became, what plays, and the crossfade into it if one runs.
#[derive(Clone)]
pub struct ModelAnimations {
    pub clips: Rc<[Animation]>,
    pub nodes: HashMap<u32, Entity>,
    pub playing: Option<Playback>,
    pub fade: Option<Fade>,
}

/// A clip playing: where it is and how it goes on.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Playback {
    pub clip: usize,
    /// Seconds into the clip.
    pub time: f32,
    /// Seconds of clip per second of `advance`; negative plays backwards.
    pub speed: f32,
    /// Wraps at the end (else holds the last pose).
    pub looping: bool,
    /// A clip that doesn't loop has told it reached its end (`Finished`);
    /// cleared once it leaves it.
    pub done: bool,
}

impl Playback {
    /// Clip `clip` from its start at speed 1.
    pub fn new(clip: usize, looping: bool) -> Self {
        Self {
            clip,
            time: 0.0,
            speed: 1.0,
            looping,
            done: false,
        }
    }
}

/// One animated property: a glTF node and what of it (`path_key`).
pub type Target = (u32, u8);

fn path_key(path: AnimationPath) -> u8 {
    match path {
        AnimationPath::Translation => 0,
        AnimationPath::Rotation => 1,
        AnimationPath::Scale => 2,
        AnimationPath::Weights => 3,
    }
}

fn key_path(key: u8) -> AnimationPath {
    match key {
        0 => AnimationPath::Translation,
        1 => AnimationPath::Rotation,
        2 => AnimationPath::Scale,
        _ => AnimationPath::Weights,
    }
}

/// A crossfade from the clip that played into the one playing now: the
/// pose is `from`'s and the new clip's mixed, the new one's weight rising
/// from 0 to 1 over `duration` seconds (as three.js's `crossFadeTo`, both
/// clips moving on meanwhile). At the end every property is the new clip's,
/// or `base`'s where it doesn't animate it.
#[derive(Debug, Clone, PartialEq)]
pub struct Fade {
    /// The clip fading out; None for the pose `base` holds (a fade begun
    /// during another fades from the mix it had reached, held still).
    pub from: Option<Playback>,
    /// Seconds of the fade gone, of `duration`.
    pub elapsed: f32,
    pub duration: f32,
    /// Each property either clip animates as it was when the fade began:
    /// what stands in for a clip that doesn't animate it.
    pub base: Rc<HashMap<Target, Vec<f32>>>,
}

/// Something an animation did in an `advance`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnimationEvent {
    /// A clip that doesn't loop reached its end (its start, backwards).
    Finished { root: Entity, clip: usize },
    /// A looping clip wrapped round.
    Looped { root: Entity, clip: usize },
}

/// `channel`'s value at `t` seconds into `out` (`channel.components` floats):
/// held at the first and last keyframes outside their range.
pub fn sample(channel: &AnimationChannel, t: f32, out: &mut [f32]) {
    // A NaN time would pass both range checks below and index before the
    // first key.
    let t = if t.is_nan() { 0.0 } else { t };
    let c = channel.components;
    let times = &channel.times;
    let cubic = channel.interpolation == Interpolation::CubicSpline;
    // A keyframe's value (the middle of in-tangent, value, out-tangent for
    // a spline).
    let value = |k: usize| {
        let at = if cubic { (k * 3 + 1) * c } else { k * c };
        &channel.values[at..at + c]
    };
    let last = times.len() - 1;
    if t <= times[0] || last == 0 {
        out[..c].copy_from_slice(value(0));
        return;
    }
    if t >= times[last] {
        out[..c].copy_from_slice(value(last));
        return;
    }
    // The keyframe interval holding t: times[k] <= t < times[k + 1].
    let k = times.partition_point(|&x| x <= t) - 1;
    let dt = times[k + 1] - times[k];
    let s = if dt > 0.0 { (t - times[k]) / dt } else { 0.0 };
    let rotation = channel.path == AnimationPath::Rotation;
    match channel.interpolation {
        Interpolation::Step => out[..c].copy_from_slice(value(k)),
        Interpolation::Linear if rotation => {
            let q0 = Quat::from_slice(value(k));
            let q1 = Quat::from_slice(value(k + 1));
            // glam's slerp takes the shorter way round (q and −q are the
            // same rotation).
            out[..4].copy_from_slice(&q0.slerp(q1, s).normalize().to_array());
        }
        Interpolation::Linear => {
            let (a, b) = (value(k), value(k + 1));
            for i in 0..c {
                out[i] = a[i] + (b[i] - a[i]) * s;
            }
        }
        Interpolation::CubicSpline => {
            // Appendix C: p(s) = (2s³ − 3s² + 1)·v0 + (s³ − 2s² + s)·dt·b0
            //                  + (−2s³ + 3s²)·v1 + (s³ − s²)·dt·a1,
            // with b0 the out-tangent of key k and a1 the in-tangent of k+1.
            let v0 = value(k);
            let v1 = value(k + 1);
            let b0 = &channel.values[(k * 3 + 2) * c..][..c];
            let a1 = &channel.values[(k + 1) * 3 * c..][..c];
            let (s2, s3) = (s * s, s * s * s);
            let h00 = 2.0 * s3 - 3.0 * s2 + 1.0;
            let h10 = s3 - 2.0 * s2 + s;
            let h01 = -2.0 * s3 + 3.0 * s2;
            let h11 = s3 - s2;
            for i in 0..c {
                out[i] = h00 * v0[i] + h10 * dt * b0[i] + h01 * v1[i] + h11 * dt * a1[i];
            }
            if rotation {
                let q = Quat::from_slice(&out[..4]).normalize();
                out[..4].copy_from_slice(&q.to_array());
            }
        }
    }
}

// Writes one property's value into node entity `e`.
fn write(world: &mut World, e: Entity, path: AnimationPath, value: &[f32]) {
    match path {
        AnimationPath::Weights => {
            let weights = MorphWeights(value.to_vec());
            match world.get_mut::<MorphWeights>(e) {
                Some(mut w) => w.set_if_neq(weights),
                None => {
                    world.insert(e, weights);
                }
            }
        }
        path => {
            let Some(mut t) = world.get_mut::<Transform>(e) else {
                return;
            };
            let mut next = *t;
            match path {
                AnimationPath::Translation => next.translation = Vec3::from_slice(value),
                AnimationPath::Rotation => next.rotation = Quat::from_slice(value).normalize(),
                AnimationPath::Scale => next.scale = Vec3::from_slice(value),
                AnimationPath::Weights => unreachable!(),
            }
            t.set_if_neq(next);
        }
    }
}

/// Poses the nodes of `anim` at `t` seconds: each channel's node entity
/// (looked up in `nodes`) gets its sampled translation, rotation, scale or
/// morph weights.
pub fn apply(world: &mut World, anim: &Animation, nodes: &HashMap<u32, Entity>, t: f32) {
    let mut buf = Vec::new();
    for channel in &anim.channels {
        let Some(&e) = nodes.get(&channel.node) else {
            continue;
        };
        buf.resize(channel.components.max(4), 0.0);
        sample(channel, t, &mut buf);
        write(world, e, channel.path, &buf[..channel.components]);
    }
}

// Every property `anim` animates, at `t`.
fn sample_clip(anim: &Animation, t: f32) -> HashMap<Target, Vec<f32>> {
    let mut out = HashMap::new();
    for channel in &anim.channels {
        let mut v = vec![0.0; channel.components.max(4)];
        sample(channel, t, &mut v);
        v.truncate(channel.components);
        out.insert((channel.node, path_key(channel.path)), v);
    }
    out
}

// A property's value on its node entity now (None without one).
fn current(world: &World, e: Entity, path: AnimationPath) -> Option<Vec<f32>> {
    match path {
        AnimationPath::Weights => world.get::<MorphWeights>(e).map(|w| w.0.clone()),
        path => world.get::<Transform>(e).map(|t| match path {
            AnimationPath::Translation => t.translation.to_array().to_vec(),
            AnimationPath::Rotation => t.rotation.to_array().to_vec(),
            _ => t.scale.to_array().to_vec(),
        }),
    }
}

/// The pose to fade from: every property clip `a` (playing) and clip `b`
/// animate — and `previous`'s, a fade under way — as it is now on the
/// nodes (or as `a` samples it, if the node has no value yet: morph
/// weights never set).
pub fn fade_base(
    world: &World,
    clips: &[Animation],
    nodes: &HashMap<u32, Entity>,
    a: &Playback,
    previous: Option<&Fade>,
    b: usize,
) -> HashMap<Target, Vec<f32>> {
    let from = clips
        .get(a.clip)
        .map(|c| sample_clip(c, a.time))
        .unwrap_or_default();
    let mut base = HashMap::new();
    for (&target, value) in previous.iter().flat_map(|f| f.base.iter()) {
        let Some(&e) = nodes.get(&target.0) else {
            continue;
        };
        let now = current(world, e, key_path(target.1)).unwrap_or_else(|| value.clone());
        base.insert(target, now);
    }
    for clip in [a.clip, b] {
        let Some(clip) = clips.get(clip) else {
            continue;
        };
        for channel in &clip.channels {
            let target = (channel.node, path_key(channel.path));
            if base.contains_key(&target) {
                continue;
            }
            let Some(&e) = nodes.get(&channel.node) else {
                continue;
            };
            let value = current(world, e, channel.path)
                .or_else(|| from.get(&target).cloned())
                .unwrap_or_else(|| vec![0.0; channel.components]);
            base.insert(target, value);
        }
    }
    base
}

// `a` toward `b` by `w` (rotations the short way round), as long as `b`
// (morph weights set with another count: what `a` lacks is `b`'s).
fn mix(a: &[f32], b: &[f32], w: f32, path: AnimationPath) -> Vec<f32> {
    if path == AnimationPath::Rotation && a.len() == 4 && b.len() == 4 {
        let q = Quat::from_slice(a).slerp(Quat::from_slice(b), w).normalize();
        return q.to_array().to_vec();
    }
    b.iter()
        .enumerate()
        .map(|(i, &b)| {
            let a = a.get(i).copied().unwrap_or(b);
            a + (b - a) * w
        })
        .collect()
}

/// Poses a crossfade: `fade.from`'s clip and `to`'s mixed by how far the
/// fade has gone.
pub fn apply_fade(
    world: &mut World,
    clips: &[Animation],
    nodes: &HashMap<u32, Entity>,
    to: &Playback,
    fade: &Fade,
) {
    let w = if fade.duration > 0.0 {
        (fade.elapsed / fade.duration).clamp(0.0, 1.0)
    } else {
        1.0
    };
    // Without a clip fading out: `base` throughout.
    let a = fade
        .from
        .and_then(|f| clips.get(f.clip).map(|c| sample_clip(c, f.time)))
        .unwrap_or_default();
    let b = clips
        .get(to.clip)
        .map(|c| sample_clip(c, to.time))
        .unwrap_or_default();
    for (target, base) in fade.base.iter() {
        let Some(&e) = nodes.get(&target.0) else {
            continue;
        };
        let path = key_path(target.1);
        let value = mix(
            a.get(target).unwrap_or(base),
            b.get(target).unwrap_or(base),
            w,
            path,
        );
        write(world, e, path, &value);
    }
}

/// Whether an animation is moving: playing at a speed, and looping or not
/// yet at its end (a held last pose doesn't change); or crossfading.
pub fn animating(world: &World) -> bool {
    let Some(s) = world.storage::<ModelAnimations>() else {
        return false;
    };
    s.data().iter().any(|a| {
        a.fade.is_some()
            || a.playing.is_some_and(|p| {
                let duration = a.clips.get(p.clip).map_or(0.0, |c| c.duration);
                p.speed != 0.0
                    && duration > 0.0
                    && (p.looping
                        || (p.speed > 0.0 && p.time < duration)
                        || (p.speed < 0.0 && p.time > 0.0))
            })
    })
}

/// Where `time` lands in a clip `duration` long: wrapped if `looping`,
/// else held at an end (0 for a non-finite time).
pub fn place(time: f32, duration: f32, looping: bool) -> f32 {
    let t = if looping && duration > 0.0 {
        let t = time.rem_euclid(duration);
        // rem_euclid can round up to the duration itself.
        if t >= duration { 0.0 } else { t }
    } else {
        time.clamp(0.0, duration)
    };
    if t.is_finite() { t } else { 0.0 }
}

// Moves `p` on by `dt` seconds: Some(true) if it wrapped round, Some(false)
// if it reached its end (once while it stays there; a clip with no length,
// or played backwards from its start, reaches it at once).
fn step(p: &mut Playback, duration: f32, dt: f32) -> Option<bool> {
    let t = p.time + dt * p.speed;
    // An infinite speed starts over rather than sticks.
    p.time = place(t, duration, p.looping);
    if !t.is_finite() {
        return None;
    }
    if p.looping {
        (duration > 0.0 && (t < 0.0 || t >= duration)).then_some(true)
    } else {
        let end = (p.speed > 0.0 && p.time == duration) || (p.speed < 0.0 && p.time == 0.0);
        if !end {
            p.done = false;
            return None;
        }
        (!std::mem::replace(&mut p.done, true)).then_some(false)
    }
}

/// Moves every playing animation (and crossfade) on by `dt` seconds and
/// poses its nodes; what clips did on the way.
pub fn advance(world: &mut World, dt: f32) -> Vec<AnimationEvent> {
    let mut events = Vec::new();
    if !dt.is_finite() {
        return events;
    }
    let playing: Vec<(Entity, ModelAnimations)> = match world.storage::<ModelAnimations>() {
        Some(s) => s
            .iter()
            .filter(|(_, a)| a.playing.is_some())
            .map(|(e, a)| (e, a.clone()))
            .collect(),
        None => return events,
    };
    for (root, anims) in playing {
        let Some(mut p) = anims.playing else {
            continue;
        };
        let Some(clip) = anims.clips.get(p.clip) else {
            continue;
        };
        match step(&mut p, clip.duration, dt) {
            Some(true) => events.push(AnimationEvent::Looped { root, clip: p.clip }),
            Some(false) => events.push(AnimationEvent::Finished { root, clip: p.clip }),
            None => {}
        }
        let mut fade = anims.fade.clone();
        if let Some(f) = &mut fade {
            if let Some(from) = &mut f.from {
                let d = anims.clips.get(from.clip).map_or(0.0, |c| c.duration);
                step(from, d, dt);
            }
            f.elapsed = (f.elapsed + dt.abs()).min(f.duration);
            // Its last pose too (weight 1: what only the old clip moved
            // back at `base`), then done.
            apply_fade(world, &anims.clips, &anims.nodes, &p, f);
        }
        match &fade {
            Some(f) if f.elapsed >= f.duration => fade = None,
            Some(_) => {}
            // Held (at its end, or not moving): nothing is written, so a
            // still scene stays unchanged.
            None if anims.playing == Some(p) => continue,
            None => apply(world, clip, &anims.nodes, p.time),
        }
        if let Some(mut a) = world.get_mut::<ModelAnimations>(root) {
            a.playing = Some(p);
            a.fade = fade;
        }
    }
    events
}

#[cfg(test)]
mod tests {
    use super::*;

    fn channel(
        path: AnimationPath,
        interpolation: Interpolation,
        times: &[f32],
        values: &[f32],
        components: usize,
    ) -> AnimationChannel {
        AnimationChannel {
            node: 0,
            path,
            interpolation,
            times: times.to_vec(),
            values: values.to_vec(),
            components,
        }
    }

    fn at(c: &AnimationChannel, t: f32) -> Vec<f32> {
        let mut out = vec![0.0; c.components.max(4)];
        sample(c, t, &mut out);
        out.truncate(c.components);
        out
    }

    // Two nodes: clip 0 moves node 0's x from 0 to 1 over 1 s; clip 1
    // moves node 0's x from 10 to 10 (held) and node 1's y from 0 to 2
    // over 2 s.
    fn two_clips() -> (World, ModelAnimations, [Entity; 2]) {
        let mut world = World::new();
        let n0 = world.spawn();
        let n1 = world.spawn();
        world.insert(n0, Transform::IDENTITY);
        world.insert(n1, Transform::IDENTITY);
        let tr = |node, times: &[f32], values: &[f32]| AnimationChannel {
            node,
            ..channel(AnimationPath::Translation, Interpolation::Linear, times, values, 3)
        };
        let clips: Rc<[Animation]> = vec![
            Animation {
                name: None,
                channels: vec![tr(0, &[0.0, 1.0], &[0.0, 0.0, 0.0, 1.0, 0.0, 0.0])],
                duration: 1.0,
            },
            Animation {
                name: None,
                channels: vec![
                    tr(0, &[0.0, 2.0], &[10.0, 0.0, 0.0, 10.0, 0.0, 0.0]),
                    tr(1, &[0.0, 2.0], &[0.0, 0.0, 0.0, 0.0, 2.0, 0.0]),
                ],
                duration: 2.0,
            },
        ]
        .into();
        let anims = ModelAnimations {
            clips,
            nodes: HashMap::from([(0, n0), (1, n1)]),
            playing: None,
            fade: None,
        };
        (world, anims, [n0, n1])
    }

    fn play(clip: usize, looping: bool) -> Playback {
        Playback::new(clip, looping)
    }

    fn pos(world: &World, e: Entity) -> Vec3 {
        world.get::<Transform>(e).unwrap().translation
    }

    #[test]
    fn events_on_the_end_and_on_wrapping() {
        let (mut world, mut anims, _) = two_clips();
        anims.playing = Some(play(0, false));
        let root = world.spawn();
        world.insert(root, anims.clone());
        assert!(advance(&mut world, 0.5).is_empty());
        assert_eq!(
            advance(&mut world, 0.6),
            [AnimationEvent::Finished { root, clip: 0 }]
        );
        assert!(advance(&mut world, 0.5).is_empty(), "held: once");
        anims.playing = Some(play(0, true));
        world.insert(root, anims.clone());
        assert!(advance(&mut world, 0.9).is_empty());
        assert_eq!(advance(&mut world, 0.2), [AnimationEvent::Looped { root, clip: 0 }]);
        let p = world.get::<ModelAnimations>(root).unwrap().playing.unwrap();
        assert!((p.time - 0.1).abs() < 1e-5, "{}", p.time);
        // Backwards: the start is the end, wrapping below 0 a loop.
        anims.playing = Some(Playback { speed: -1.0, time: 0.5, ..play(0, false) });
        world.insert(root, anims.clone());
        assert_eq!(advance(&mut world, 1.0), [AnimationEvent::Finished { root, clip: 0 }]);
        anims.playing = Some(Playback { speed: -1.0, time: 0.5, ..play(0, true) });
        world.insert(root, anims);
        assert_eq!(advance(&mut world, 1.0), [AnimationEvent::Looped { root, clip: 0 }]);
    }

    // A fade begun with `fade_base` as `crossfadeAnimation` begins one.
    fn crossfade(world: &mut World, root: Entity, to: usize, seconds: f32) {
        let a = world.get::<ModelAnimations>(root).unwrap().clone();
        let from = a.playing.unwrap();
        let base = fade_base(world, &a.clips, &a.nodes, &from, a.fade.as_ref(), to);
        let fade = Fade {
            from: if a.fade.is_some() { None } else { Some(from) },
            elapsed: 0.0,
            duration: seconds,
            base: base.into(),
        };
        let p = play(to, true);
        apply_fade(world, &a.clips, &a.nodes, &p, &fade);
        let mut m = world.get_mut::<ModelAnimations>(root).unwrap();
        m.playing = Some(p);
        m.fade = Some(fade);
    }

    // Fading out of clip 1 (the only one moving node 1): once over, node 1
    // is back where it stood as the fade began — however the last step
    // falls.
    #[test]
    fn a_fade_ends_at_its_full_weight() {
        for last in [0.2, 5.0] {
            let (mut world, mut anims, [_, n1]) = two_clips();
            anims.playing = Some(play(1, true));
            let root = world.spawn();
            world.insert(root, anims);
            advance(&mut world, 1.0);
            assert!((pos(&world, n1).y - 1.0).abs() < 1e-5);
            crossfade(&mut world, root, 0, 1.0);
            advance(&mut world, 0.9);
            advance(&mut world, last);
            assert!(world.get::<ModelAnimations>(root).unwrap().fade.is_none());
            assert!((pos(&world, n1).y - 1.0).abs() < 1e-5, "{last}: {}", pos(&world, n1));
        }
    }

    // A crossfade during one starts from the mix reached: no jump.
    #[test]
    fn a_fade_during_a_fade_starts_where_it_is() {
        let (mut world, mut anims, [n0, _]) = two_clips();
        anims.playing = Some(play(0, true));
        let root = world.spawn();
        world.insert(root, anims);
        crossfade(&mut world, root, 1, 1.0);
        advance(&mut world, 0.5);
        let mid = pos(&world, n0);
        assert!(mid.x > 1.0 && mid.x < 10.0, "{mid}");
        crossfade(&mut world, root, 0, 1.0);
        assert_eq!(pos(&world, n0), mid, "no jump");
        advance(&mut world, 1.0);
        let a = world.get::<ModelAnimations>(root).unwrap();
        let t = a.playing.unwrap().time;
        assert!((pos(&world, n0).x - t).abs() < 1e-5, "clip 0 alone at its end");
    }

    // Played backwards from its start (or a clip with no length): finished
    // at once, and once.
    #[test]
    fn finished_at_once_when_already_at_the_end() {
        let (mut world, mut anims, _) = two_clips();
        anims.playing = Some(Playback { speed: -1.0, ..play(0, false) });
        let root = world.spawn();
        world.insert(root, anims);
        assert_eq!(advance(&mut world, 0.1), [AnimationEvent::Finished { root, clip: 0 }]);
        assert!(advance(&mut world, 0.1).is_empty());
    }

    #[test]
    fn mixing_weights_of_another_count_keeps_the_target_count() {
        let m = mix(&[1.0], &[0.0, 0.0], 0.5, AnimationPath::Weights);
        assert_eq!(m, [0.5, 0.0]);
    }

    #[test]
    fn place_wraps_or_holds() {
        assert_eq!(place(2.5, 1.0, true), 0.5);
        assert_eq!(place(-0.25, 1.0, true), 0.75);
        assert_eq!(place(2.5, 1.0, false), 1.0);
        assert_eq!(place(-1.0, 1.0, false), 0.0);
        assert_eq!(place(f32::INFINITY, 1.0, true), 0.0);
        assert!(place(-1e-9, 1.0, true) < 1.0, "never the duration itself");
    }

    // Halfway through a 1 s fade from clip 0 (at 0.5 s: x = 0.5) into clip 1
    // (x = 10; node 1 only in clip 1, from its pose at the start, y = 0).
    #[test]
    fn a_crossfade_mixes_the_clips_then_hands_over() {
        let (mut world, mut anims, [n0, n1]) = two_clips();
        let from = Playback { time: 0.25, ..play(0, true) };
        apply(&mut world, &anims.clips[0], &anims.nodes, from.time);
        let base = fade_base(&world, &anims.clips, &anims.nodes, &from, None, 1);
        assert_eq!(base.len(), 2, "both nodes' translations");
        let fade = Fade {
            from: Some(from),
            elapsed: 0.0,
            duration: 1.0,
            base: base.into(),
        };
        let to = play(1, true);
        apply_fade(&mut world, &anims.clips, &anims.nodes, &to, &fade);
        assert_eq!(pos(&world, n0).x, 0.25, "no jump as it starts");
        anims.playing = Some(to);
        anims.fade = Some(fade);
        let root = world.spawn();
        world.insert(root, anims);
        assert!(animating(&world));
        advance(&mut world, 0.25);
        // from at 0.5 (x 0.5), to at 0.25 (x 10, y 0.25), weight 0.25.
        let p = pos(&world, n0);
        assert!((p.x - (0.5 + (10.0 - 0.5) * 0.25)).abs() < 1e-4, "{p}");
        let q = pos(&world, n1);
        assert!((q.y - 0.25 * 0.25).abs() < 1e-5, "{q}");
        advance(&mut world, 0.75);
        let a = world.get::<ModelAnimations>(root).unwrap();
        assert!(a.fade.is_none(), "over");
        assert_eq!(pos(&world, n0).x, 10.0);
        assert!((pos(&world, n1).y - 1.0).abs() < 1e-5);
    }

    #[test]
    fn linear_step_and_clamping() {
        let c = channel(
            AnimationPath::Translation,
            Interpolation::Linear,
            &[1.0, 3.0],
            &[0.0, 0.0, 0.0, 2.0, 4.0, -2.0],
            3,
        );
        assert_eq!(at(&c, 2.0), [1.0, 2.0, -1.0]);
        assert_eq!(at(&c, 0.0), [0.0, 0.0, 0.0], "held before the first key");
        assert_eq!(at(&c, 9.0), [2.0, 4.0, -2.0], "held after the last");
        let step = AnimationChannel {
            interpolation: Interpolation::Step,
            ..c
        };
        assert_eq!(at(&step, 2.9), [0.0, 0.0, 0.0]);
        assert_eq!(at(&step, 3.0), [2.0, 4.0, -2.0]);
    }

    // A NaN time (an infinite speed, a NaN step) holds the first key
    // instead of indexing before it.
    #[test]
    fn a_nan_time_is_harmless() {
        let c = channel(
            AnimationPath::Translation,
            Interpolation::Linear,
            &[1.0, 3.0],
            &[0.0, 0.0, 0.0, 2.0, 4.0, -2.0],
            3,
        );
        assert_eq!(at(&c, f32::NAN), [0.0, 0.0, 0.0]);
    }

    #[test]
    fn rotations_slerp_the_short_way() {
        let a = Quat::from_rotation_y(0.0);
        // The same 90° turn, stored as −q: still a 45° halfway point.
        let b = -Quat::from_rotation_y(std::f32::consts::FRAC_PI_2);
        let mut values = a.to_array().to_vec();
        values.extend(b.to_array());
        let c = channel(
            AnimationPath::Rotation,
            Interpolation::Linear,
            &[0.0, 1.0],
            &values,
            4,
        );
        let q = Quat::from_slice(&at(&c, 0.5));
        let expected = Quat::from_rotation_y(std::f32::consts::FRAC_PI_4);
        assert!(q.angle_between(expected) < 1e-4, "{q:?}");
    }

    #[test]
    fn cubic_splines_follow_their_tangents() {
        // One component: v0 = 0, v1 = 1 over dt = 2; out-tangent of key 0
        // and in-tangent of key 1 both 0.5 (slope per second).
        let values = [0.0, 0.0, 0.5, 0.5, 1.0, 0.0];
        let c = channel(
            AnimationPath::Weights,
            Interpolation::CubicSpline,
            &[0.0, 2.0],
            &values,
            1,
        );
        // Tangents dt·0.5 = 1 = the chord: the spline is the straight line.
        for (t, v) in [(0.5, 0.25), (1.0, 0.5), (1.5, 0.75)] {
            assert!((at(&c, t)[0] - v).abs() < 1e-6, "{t}: {:?}", at(&c, t));
        }
        // Flat tangents ease in and out: below the line early, above late.
        let flat = AnimationChannel {
            values: vec![0.0, 0.0, 0.0, 0.0, 1.0, 0.0],
            ..c
        };
        assert!(at(&flat, 0.5)[0] < 0.25 && at(&flat, 1.5)[0] > 0.75);
        assert!((at(&flat, 1.0)[0] - 0.5).abs() < 1e-6);
        assert_eq!(at(&flat, 2.0), [1.0], "the value, not a tangent, at a key");
    }
}
