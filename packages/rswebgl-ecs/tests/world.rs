// Host tests: `cargo test -p rswebgl-ecs`.

use rswebgl_ecs::{Entity, Tick, World};

#[derive(Debug, Clone, Copy, PartialEq)]
struct Pos(f32);
#[derive(Debug, Clone, Copy, PartialEq)]
struct Vel(f32);
#[derive(Debug, PartialEq)]
struct Name(&'static str);

#[test]
fn a_despawned_entity_stays_dead_when_its_slot_is_reused() {
    let mut w = World::new();
    let a = w.spawn();
    w.insert(a, Pos(1.0));
    assert!(w.despawn(a));
    assert!(!w.despawn(a), "twice");
    assert!(!w.is_alive(a));

    // Churn until a's slot comes back with a new generation.
    let mut reused = None;
    for _ in 0..10_000 {
        let b = w.spawn();
        if b.0.index() == a.0.index() {
            reused = Some(b);
            break;
        }
        w.despawn(b);
    }
    let b = reused.expect("slot reused");
    w.insert(b, Pos(2.0));
    assert_eq!(w.get::<Pos>(a), None, "the old handle doesn't reach b");
    assert_eq!(w.get::<Pos>(b), Some(&Pos(2.0)));
    assert_eq!(
        w.insert(a, Pos(3.0)),
        None,
        "insert on a dead entity is a no-op"
    );
    assert_eq!(w.get::<Pos>(b), Some(&Pos(2.0)));
}

#[test]
fn removing_a_row_keeps_the_others_intact() {
    let mut w = World::new();
    let es: Vec<Entity> = (0..5).map(|_| w.spawn()).collect();
    for (i, &e) in es.iter().enumerate() {
        w.insert(e, Pos(i as f32));
    }
    assert_eq!(w.insert(es[1], Pos(10.0)), Some(Pos(1.0)), "replace");
    assert_eq!(w.remove::<Pos>(es[1]), Some(Pos(10.0)));
    assert_eq!(w.remove::<Pos>(es[1]), None);
    w.despawn(es[3]);
    let s = w.storage::<Pos>().unwrap();
    assert_eq!(s.len(), 3);
    for (i, &e) in es.iter().enumerate() {
        let want = (i != 1 && i != 3).then_some(Pos(i as f32));
        assert_eq!(s.get(e).copied(), want, "entity {i}");
    }
    // Rows stay dense and agree with the entity list.
    for (row, &e) in s.entities().iter().enumerate() {
        assert_eq!(s.row(e), Some(row as u32));
    }
    assert!(w.get::<Name>(es[0]).is_none(), "never-used component");
}

// The system pattern from `advance_tick`: sees what changed since its last
// run, including writes made after it ran — but not its own writes.
#[test]
fn systems_see_changes_since_their_last_run() {
    let mut w = World::new();
    let a = w.spawn();
    let b = w.spawn();
    w.insert(a, Pos(0.0));
    w.insert(b, Pos(0.0));

    let mut last_run: Tick = 0;
    let mut run = |w: &mut World| -> Vec<Entity> {
        let now = w.advance_tick();
        let seen = w
            .storage::<Pos>()
            .unwrap()
            .iter_changed(last_run)
            .map(|(e, _)| e)
            .collect();
        // The system's own write: new to others, not to itself.
        w.storage_mut::<Pos>().get_mut(a, now).unwrap().0 += 1.0;
        last_run = now;
        seen
    };

    assert_eq!(run(&mut w), [a, b], "everything is new at first");
    assert_eq!(run(&mut w), [], "own writes aren't seen");

    // Reading through `Mut` doesn't count; writing does.
    let _ = w.get_mut::<Pos>(a).unwrap().0;
    w.get_mut::<Pos>(b).unwrap().0 = 5.0;
    assert_eq!(run(&mut w), [b]);

    // Writing an equal value with set_if_neq isn't a change.
    w.get_mut::<Pos>(b).unwrap().set_if_neq(Pos(5.0));
    assert_eq!(run(&mut w), []);
    w.get_mut::<Pos>(b).unwrap().set_if_neq(Pos(6.0));
    assert_eq!(run(&mut w), [b]);
}

#[test]
fn another_system_sees_a_systems_writes() {
    let mut w = World::new();
    let e = w.spawn();
    w.insert(e, Pos(0.0));
    let (mut first_last, mut second_last): (Tick, Tick) = (0, 0);
    for _ in 0..2 {
        // First writes every frame...
        let now = w.advance_tick();
        w.storage_mut::<Pos>().get_mut(e, now).unwrap().0 += 1.0;
        first_last = now;
        // ...second reads after it.
        let now = w.advance_tick();
        let seen = w
            .storage::<Pos>()
            .unwrap()
            .iter_changed(second_last)
            .count();
        assert_eq!(seen, 1);
        second_last = now;
    }
    assert!(first_last < second_last);
}

#[test]
fn join_yields_entities_with_both_components() {
    let mut w = World::new();
    let both = w.spawn();
    let only_pos = w.spawn();
    let only_vel = w.spawn();
    w.insert(both, Pos(1.0));
    w.insert(both, Vel(2.0));
    w.insert(only_pos, Pos(3.0));
    w.insert(only_vel, Vel(4.0));
    let got: Vec<_> = w.join::<Vel, Pos>().collect();
    assert_eq!(got, [(both, &Vel(2.0), &Pos(1.0))]);
    assert_eq!(w.join::<Name, Pos>().count(), 0, "unknown component");
}

#[test]
#[should_panic(expected = "two different component types")]
fn storages_mut_rejects_the_same_type_twice() {
    let mut w = World::new();
    let _ = w.storages_mut::<Pos, Pos>();
}

// Removals are logged with their tick: each system sees those after its own
// last run, whenever it looks; trimming drops only what everyone has seen.
#[test]
fn removals_are_seen_after_each_last_run() {
    let mut w = World::new();
    let [a, b] = [w.spawn(), w.spawn()];
    w.insert(a, Pos(0.0));
    w.insert(b, Pos(0.0));
    let early = w.advance_tick(); // a system that ran before the removals
    w.remove::<Pos>(a);
    w.despawn(b);
    let late = w.advance_tick(); // one that ran after them
    let s = w.storage::<Pos>().unwrap();
    assert_eq!(s.removed_since(early).collect::<Vec<_>>(), [a, b]);
    assert!(s.any_removed_since(early));
    assert_eq!(s.removed_since(late).count(), 0);
    w.trim_removed(early);
    assert_eq!(
        w.storage::<Pos>().unwrap().removed_since(early).count(),
        2,
        "not seen by all yet"
    );
    w.trim_removed(late);
    assert_eq!(w.storage::<Pos>().unwrap().removed_since(0).count(), 0);
}

// The tick rule: a system's writes stamped with its own `now` are new to
// the systems after it, not to itself next run; the app's writes are new to
// everyone.
#[test]
fn a_system_does_not_see_its_own_writes() {
    let mut w = World::new();
    let e = w.spawn();
    w.insert(e, Pos(0.0));
    let mut mine = 0; // the system's last_run
    let mut other = 0; // a system that runs after it
    for frame in 0..3 {
        let now = w.advance_tick();
        let changed = w.storage::<Pos>().unwrap().changed_since(mine);
        assert_eq!(changed, frame == 0, "frame {frame}");
        w.get_mut_at::<Pos>(e, now).unwrap().0 += 1.0;
        w.insert_at(e, Vel(1.0), now);
        mine = now;
        let now = w.advance_tick();
        assert!(
            w.storage::<Pos>().unwrap().changed_since(other),
            "frame {frame}"
        );
        assert!(w.storage::<Vel>().unwrap().changed_since(other));
        other = now;
    }
    // An app write in between: new to the system.
    w.get_mut::<Pos>(e).unwrap().0 = 0.0;
    assert!(w.storage::<Pos>().unwrap().changed_since(mine));
}

// O(1) checks: values changed (`changed_since`, adds included) and rows
// added or removed (`structure_changed_since`).
#[test]
fn storage_level_change_checks() {
    let mut w = World::new();
    let e = w.spawn();
    w.insert(e, Pos(0.0));
    let t = w.advance_tick();
    let s = w.storage::<Pos>().unwrap();
    assert!(!s.changed_since(t) && !s.structure_changed_since(t));
    // A read through Mut, or an equal set_if_neq, changes nothing.
    {
        let s = w.storage_mut::<Pos>();
        let mut m = s.get_mut(e, 5).unwrap();
        let _ = m.0;
        m.set_if_neq(Pos(0.0));
    }
    assert!(!w.storage::<Pos>().unwrap().changed_since(t));
    w.get_mut::<Pos>(e).unwrap().0 = 1.0;
    let s = w.storage::<Pos>().unwrap();
    assert!(s.changed_since(t) && !s.structure_changed_since(t));
    let t = w.advance_tick();
    w.remove::<Pos>(e);
    let s = w.storage::<Pos>().unwrap();
    assert!(!s.changed_since(t) && s.structure_changed_since(t));
    // Through iter_mut too.
    let f = w.spawn();
    w.insert(f, Pos(0.0));
    let t = w.advance_tick();
    let now = w.advance_tick();
    for (_, mut p) in w.storage_mut::<Pos>().iter_mut(now) {
        p.0 = 2.0;
    }
    assert!(w.storage::<Pos>().unwrap().changed_since(t));
    assert_eq!(w.storage::<Pos>().unwrap().newest_change(), now);
}
