//! Parent/child transforms: `TransformSystem` turns `Transform` + `Parent` into
//! `WorldMatrix`, touching only what moved.

use std::collections::HashMap;
use std::fmt;

use luxel_ecs::{Entity, Tick, World};

use crate::transform::{Parent, Transform, WorldMatrix};

const NONE: u32 = u32::MAX;

// One entity in parent-first order; `parent` is the parent's position in the
// same order (NONE for a root).
#[derive(Clone, Copy)]
struct Node {
    entity: Entity,
    parent: u32,
}

/// Computes `WorldMatrix` for every entity with a `Transform`, parents before
/// children.
///
/// The parent-first order is cached and rebuilt only when the hierarchy
/// changes: a `Parent` set or removed, a `Transform` added or removed, an
/// entity despawned. A frame then recomputes the entities whose `Transform`
/// changed since the last update, plus their subtrees — a frame where nothing
/// moved writes nothing.
///
/// Call `update` once per frame. Removals are seen through the world's
/// removal logs after `last_run`: trim them (`World::trim_removed`) no
/// further than that.
///
/// Entities caught in a `Parent` cycle (possible only by inserting `Parent`
/// directly instead of through [`set_parent`]) are left out: they get no
/// `WorldMatrix` updates, and [`TransformSystem::skipped`] counts them.
pub struct TransformSystem {
    last_run: Tick,
    order: Vec<Node>,
    skipped: usize,
    // Scratch, kept to avoid reallocating every frame.
    dirty: Vec<bool>,
    parent_rows: Vec<u32>,
    child_start: Vec<u32>,
    child_rows: Vec<u32>,
    rows: Vec<u32>,
}

impl Default for TransformSystem {
    fn default() -> Self {
        Self::new()
    }
}

impl TransformSystem {
    pub fn new() -> Self {
        Self {
            last_run: 0,
            order: Vec::new(),
            skipped: 0,
            dirty: Vec::new(),
            parent_rows: Vec::new(),
            child_start: Vec::new(),
            child_rows: Vec::new(),
            rows: Vec::new(),
        }
    }

    /// Updates world matrices; returns how many were recomputed.
    pub fn update(&mut self, world: &mut World) -> usize {
        let now = world.advance_tick();
        let all = self.needs_rebuild(world);
        if all {
            self.rebuild(world, now);
        }
        let count = self.recompute(world, now, all);
        self.last_run = now;
        count
    }

    /// The tick of its last `update` (removals logged after it are still
    /// to be seen).
    pub fn last_run(&self) -> Tick {
        self.last_run
    }

    /// Entities with a `Transform` left out of the last rebuild because their
    /// `Parent` chain loops.
    pub fn skipped(&self) -> usize {
        self.skipped
    }

    fn needs_rebuild(&self, world: &World) -> bool {
        let transforms_len = world.storage::<Transform>().map_or(0, |s| s.len());
        if self.order.len() + self.skipped != transforms_len {
            return true;
        }
        // All O(1): a Transform or Parent added, removed or (Parent) changed,
        // a WorldMatrix taken away.
        let since = self.last_run;
        world
            .storage::<Transform>()
            .is_some_and(|s| s.structure_changed_since(since))
            || world
                .storage::<Parent>()
                .is_some_and(|s| s.changed_since(since) || s.structure_changed_since(since))
            || world
                .storage::<WorldMatrix>()
                .is_some_and(|s| s.any_removed_since(since))
    }

    fn rebuild(&mut self, world: &mut World, now: Tick) {
        self.order.clear();
        self.skipped = 0;
        let Some(transforms) = world.storage::<Transform>() else {
            return;
        };
        let n = transforms.len();

        // Each row's parent row: a live parent with a Transform, else a root.
        self.parent_rows.clear();
        self.parent_rows
            .extend(transforms.entities().iter().map(|&e| {
                world
                    .get::<Parent>(e)
                    .filter(|p| p.0 != e)
                    .and_then(|p| transforms.row(p.0))
                    .unwrap_or(NONE)
            }));

        // Children grouped by parent row (counting sort).
        self.child_start.clear();
        self.child_start.resize(n + 1, 0);
        for &p in &self.parent_rows {
            if p != NONE {
                self.child_start[p as usize + 1] += 1;
            }
        }
        for i in 0..n {
            self.child_start[i + 1] += self.child_start[i];
        }
        self.child_rows.clear();
        self.child_rows.resize(self.child_start[n] as usize, 0);
        let mut fill = self.child_start.clone();
        for (child, &p) in self.parent_rows.iter().enumerate() {
            if p != NONE {
                self.child_rows[fill[p as usize] as usize] = child as u32;
                fill[p as usize] += 1;
            }
        }

        // Breadth-first from the roots: parents always precede children.
        // Rows on a cycle are never reached from a root.
        self.rows.clear();
        let entities = transforms.entities();
        for (row, &p) in self.parent_rows.iter().enumerate() {
            if p == NONE {
                self.rows.push(row as u32);
                self.order.push(Node {
                    entity: entities[row],
                    parent: NONE,
                });
            }
        }
        let mut i = 0;
        while i < self.rows.len() {
            let row = self.rows[i] as usize;
            let (start, end) = (self.child_start[row], self.child_start[row + 1]);
            for &child in &self.child_rows[start as usize..end as usize] {
                self.rows.push(child);
                self.order.push(Node {
                    entity: entities[child as usize],
                    parent: i as u32,
                });
            }
            i += 1;
        }
        self.skipped = n - self.order.len();

        let missing: Vec<Entity> = self
            .order
            .iter()
            .map(|node| node.entity)
            .filter(|&e| !world.has::<WorldMatrix>(e))
            .collect();
        // At its own tick: not a change it rebuilds for next time.
        for e in missing {
            world.insert_at(e, WorldMatrix::default(), now);
        }
    }

    fn recompute(&mut self, world: &mut World, now: Tick, all: bool) -> usize {
        let (transforms, worlds) = world.storages_mut::<Transform, WorldMatrix>();
        self.dirty.clear();
        self.dirty.resize(self.order.len(), false);
        let mut count = 0;
        for (i, node) in self.order.iter().enumerate() {
            let Some(row) = transforms.row(node.entity) else {
                continue;
            };
            let parent_moved = node.parent != NONE && self.dirty[node.parent as usize];
            if !(all || parent_moved || transforms.changed_tick(row) > self.last_run) {
                continue;
            }
            self.dirty[i] = true;
            let local = transforms.data()[row as usize].matrix();
            let m = match node.parent {
                NONE => local,
                p => {
                    let parent = self.order[p as usize].entity;
                    worlds.get(parent).map_or(local, |pm| pm.0 * local)
                }
            };
            if let Some(mut wm) = worlds.get_mut(node.entity, now) {
                wm.0 = m;
                count += 1;
            }
        }
        count
    }
}

/// Why [`set_parent`] refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HierarchyError {
    /// The child or the parent isn't alive.
    DeadEntity,
    /// The parent is the child itself or one of its descendants.
    Cycle,
}

impl fmt::Display for HierarchyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::DeadEntity => "the child or the parent entity is not alive",
            Self::Cycle => "the parent is the child itself or one of its descendants",
        })
    }
}

impl std::error::Error for HierarchyError {}

/// Makes `child`'s transform relative to `parent`'s, or a root with `None`.
/// Refuses cycles. Setting the parent it already has changes nothing (and
/// doesn't trigger a hierarchy rebuild).
pub fn set_parent(
    world: &mut World,
    child: Entity,
    parent: Option<Entity>,
) -> Result<(), HierarchyError> {
    if !world.is_alive(child) {
        return Err(HierarchyError::DeadEntity);
    }
    let Some(parent) = parent else {
        world.remove::<Parent>(child);
        return Ok(());
    };
    if !world.is_alive(parent) {
        return Err(HierarchyError::DeadEntity);
    }
    // Walk up from the new parent; meeting the child means a loop. Bounded in
    // case a cycle was already inserted by hand.
    let mut at = parent;
    for _ in 0..=world.len() {
        if at == child {
            return Err(HierarchyError::Cycle);
        }
        match world.get::<Parent>(at) {
            Some(p) => at = p.0,
            None => break,
        }
    }
    if world.get::<Parent>(child) != Some(&Parent(parent)) {
        world.insert(child, Parent(parent));
    }
    Ok(())
}

/// The entities whose `Parent` is `e`. Scans every `Parent`, so it's for
/// occasional queries, not per-frame loops.
pub fn children(world: &World, e: Entity) -> Vec<Entity> {
    world.storage::<Parent>().map_or_else(Vec::new, |parents| {
        parents
            .iter()
            .filter(|&(_, p)| p.0 == e)
            .map(|(child, _)| child)
            .collect()
    })
}

/// Despawns `e` and everything under it; returns how many entities that was
/// (0 if `e` was already dead).
pub fn despawn_recursive(world: &mut World, e: Entity) -> usize {
    if !world.is_alive(e) {
        return 0;
    }
    let mut by_parent: HashMap<Entity, Vec<Entity>> = HashMap::new();
    if let Some(parents) = world.storage::<Parent>() {
        for (child, p) in parents.iter() {
            by_parent.entry(p.0).or_default().push(child);
        }
    }
    let mut stack = vec![e];
    let mut count = 0;
    while let Some(at) = stack.pop() {
        // `despawn` is false for an entity seen twice (a hand-made cycle).
        if world.despawn(at) {
            count += 1;
            if let Some(kids) = by_parent.get(&at) {
                stack.extend(kids);
            }
        }
    }
    count
}

/// `root` and everything under it, parents before children. Builds a map of
/// children from every `Parent` once, so it's for occasional queries.
pub fn descendants(world: &World, root: Entity) -> Vec<Entity> {
    if !world.is_alive(root) {
        return Vec::new();
    }
    let mut by_parent: HashMap<Entity, Vec<Entity>> = HashMap::new();
    if let Some(parents) = world.storage::<Parent>() {
        for (child, p) in parents.iter() {
            by_parent.entry(p.0).or_default().push(child);
        }
    }
    let mut out = vec![root];
    let mut i = 0;
    // Bounded against a hand-made cycle through `root`.
    while i < out.len() && out.len() <= world.len() {
        if let Some(kids) = by_parent.get(&out[i]) {
            out.extend(kids.iter().filter(|&&k| k != root));
        }
        i += 1;
    }
    out
}
