// Host tests: `cargo test -p luxel`.

use glam::Vec3;
use luxel::{
    Bounds, Camera, Transform, TransformSystem, WorldMatrix, frame_box, set_parent, world_bounds,
    world_to_ndc,
};
use luxel_ecs::World;

#[test]
fn a_subtree_is_framed_whole() {
    let mut w = World::new();
    let root = w.spawn();
    w.insert(root, Transform::from_translation(Vec3::new(10.0, 0.0, 0.0)));
    let child = w.spawn();
    w.insert(child, Transform::from_translation(Vec3::new(0.0, 4.0, 0.0)));
    w.insert(child, Bounds::new(Vec3::splat(-1.0), Vec3::splat(1.0)));
    set_parent(&mut w, child, Some(root)).unwrap();
    w.insert(root, Bounds::new(Vec3::splat(-1.0), Vec3::splat(1.0)));
    let camera = w.spawn();
    w.insert(camera, Camera::perspective(0.8, 2.0, 0.1, 1000.0));
    w.insert(camera, Transform::IDENTITY); // looks down −Z
    let mut sys = TransformSystem::new();
    sys.update(&mut w);

    let (min, max) = world_bounds(&w, root).unwrap();
    assert_eq!(
        (min, max),
        (Vec3::new(9.0, -1.0, -1.0), Vec3::new(11.0, 5.0, 1.0))
    );
    let (center, distance) = frame_box(&mut w, camera, min, max, 1.0).unwrap();
    assert_eq!(center, Vec3::new(10.0, 2.0, 0.0));
    sys.update(&mut w);
    let eye = w.get::<WorldMatrix>(camera).unwrap().0.w_axis.truncate();
    assert!(eye.abs_diff_eq(center + Vec3::Z * distance, 1e-3), "{eye}");
    // Every corner on screen, the sphere touching the narrower (vertical)
    // edge.
    for x in [min.x, max.x] {
        for y in [min.y, max.y] {
            for z in [min.z, max.z] {
                let p = world_to_ndc(&w, camera, Vec3::new(x, y, z)).unwrap();
                assert!(p.x.abs() <= 1.0 && p.y.abs() <= 1.0 && p.z > 0.0, "{p}");
            }
        }
    }
    // The far plane reaches past the box, the near plane stays in front.
    let Camera { projection, .. } = *w.get::<Camera>(camera).unwrap();
    let luxel::Projection::Perspective { near, far, .. } = projection else {
        panic!()
    };
    let r = (max - min).length() * 0.5;
    assert!(near <= distance - r + 1e-3 && far >= distance + r - 1e-3);
}

#[test]
fn nothing_with_bounds_is_none() {
    let mut w = World::new();
    let e = w.spawn();
    w.insert(e, Transform::IDENTITY);
    TransformSystem::new().update(&mut w);
    assert!(world_bounds(&w, e).is_none());
}
