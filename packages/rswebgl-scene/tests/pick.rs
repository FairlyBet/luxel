// Host tests: `cargo test -p rswebgl-scene`.

use glam::{Mat4, Vec2, Vec3};
use rswebgl::draw::DrawCommand;
use rswebgl::ids::{ParamGroupId, ProgramId, StateId, VertexArrayId};
use rswebgl_ecs::World;
use rswebgl_scene::{
    Bounds, Camera, MeshRenderer, Ray, Transform, TransformSystem, Visible, camera_ray,
    pick_bounds, ray_aabb, world_to_ndc,
};

fn ray(o: [f32; 3], d: [f32; 3]) -> Ray {
    Ray {
        origin: Vec3::from(o),
        dir: Vec3::from(d).normalize(),
    }
}

#[test]
fn boxes() {
    let (min, max) = (Vec3::splat(-1.0), Vec3::splat(1.0));
    assert_eq!(
        ray_aabb(&ray([0.0, 0.0, 5.0], [0.0, 0.0, -1.0]), min, max),
        Some(4.0)
    );
    assert_eq!(
        ray_aabb(&ray([0.0, 0.0, 0.0], [1.0, 0.0, 0.0]), min, max),
        Some(0.0),
        "inside"
    );
    assert_eq!(
        ray_aabb(&ray([0.0, 0.0, 5.0], [0.0, 0.0, 1.0]), min, max),
        None,
        "behind"
    );
    assert_eq!(
        ray_aabb(&ray([3.0, 0.0, 5.0], [0.0, 0.0, -1.0]), min, max),
        None,
        "beside"
    );
    // Axis-parallel rays (infinite reciprocals) still work.
    assert_eq!(
        ray_aabb(&ray([1.0, 0.0, 5.0], [0.0, 0.0, -1.0]), min, max),
        Some(4.0),
        "on a face"
    );
}

#[test]
fn camera_rays_go_through_the_screen_point() {
    let cam = Camera::perspective(std::f32::consts::FRAC_PI_2, 1.0, 1.0, 100.0);
    let view = Mat4::from_translation(Vec3::new(0.0, 0.0, 10.0)).inverse();
    let vp = cam.projection_matrix() * view;
    let center = Ray::through(&vp, Vec2::ZERO);
    assert!(
        center.origin.abs_diff_eq(Vec3::new(0.0, 0.0, 9.0), 1e-4),
        "near plane"
    );
    assert!(center.dir.abs_diff_eq(Vec3::NEG_Z, 1e-5));
    // 90° fov: the right edge of the screen is 45° off the axis.
    let right = Ray::through(&vp, Vec2::new(1.0, 0.0));
    assert!(
        right
            .dir
            .abs_diff_eq(Vec3::new(1.0, 0.0, -1.0).normalize(), 1e-4)
    );
    // A ray moved into a mesh's space keeps `t` meaning the same point.
    let m = Mat4::from_scale(Vec3::splat(2.0));
    let local = center.transformed(&m.inverse());
    assert!(
        m.transform_point3(local.at(3.0))
            .abs_diff_eq(center.at(3.0), 1e-4)
    );
}

// A camera at z = 10 looking down -Z (90° fov), and two unit boxes on the
// axis at z = 0 and z = -5.
fn world() -> (World, rswebgl_ecs::Entity, [rswebgl_ecs::Entity; 2]) {
    let mut w = World::new();
    let camera = w.spawn();
    w.insert(
        camera,
        Camera::perspective(std::f32::consts::FRAC_PI_2, 1.0, 1.0, 100.0),
    );
    w.insert(
        camera,
        Transform::from_translation(Vec3::new(0.0, 0.0, 10.0)),
    );
    let boxes = [0.0, -5.0].map(|z| {
        let e = w.spawn();
        w.insert(e, Transform::from_translation(Vec3::new(0.0, 0.0, z)));
        w.insert(e, Bounds::new(Vec3::splat(-0.5), Vec3::splat(0.5)));
        w.insert(
            e,
            MeshRenderer {
                program: ProgramId::NULL,
                state: StateId::NULL,
                vao: VertexArrayId::NULL,
                material: ParamGroupId::NULL,
                command: DrawCommand::arrays(rswebgl::draw::DrawMode::Triangles, 0, 3),
                transparent: false,
                transmissive: false,
            },
        );
        e
    });
    TransformSystem::new().update(&mut w);
    (w, camera, boxes)
}

#[test]
fn world_points_project_to_the_screen() {
    let (w, camera, _) = world();
    let center = world_to_ndc(&w, camera, Vec3::ZERO).unwrap();
    assert!(
        center.abs_diff_eq(Vec3::new(0.0, 0.0, 10.0), 1e-4),
        "{center}"
    );
    // 90° fov: 10 units off-axis at depth 10 is the screen's edge.
    let edge = world_to_ndc(&w, camera, Vec3::new(10.0, 5.0, 0.0)).unwrap();
    assert!(edge.abs_diff_eq(Vec3::new(1.0, 0.5, 10.0), 1e-4), "{edge}");
    let behind = world_to_ndc(&w, camera, Vec3::new(0.0, 0.0, 20.0)).unwrap();
    assert!(behind.z < 0.0);
    // Through the projected point, back to it.
    let ray = camera_ray(&w, camera, Vec2::new(1.0, 0.5)).unwrap();
    let t = -ray.origin.z / ray.dir.z; // to z = 0
    assert!(ray.at(t).abs_diff_eq(Vec3::new(10.0, 5.0, 0.0), 1e-3));
    assert!(world_to_ndc(&w, rswebgl_ecs::Entity::NULL, Vec3::ZERO).is_none());
}

#[test]
fn bounds_picking_finds_the_nearest_visible_box() {
    let (mut w, camera, [near, far]) = world();
    let hit = pick_bounds(&w, camera, Vec2::ZERO).unwrap();
    assert_eq!(hit.entity, near);
    // From the camera: its box's front face is 9.5 away.
    assert!((hit.distance - 9.5).abs() < 1e-3, "{}", hit.distance);
    assert!(hit.point.abs_diff_eq(Vec3::new(0.0, 0.0, 0.5), 1e-3));
    w.insert(near, Visible(false));
    assert_eq!(pick_bounds(&w, camera, Vec2::ZERO).unwrap().entity, far);
    assert!(
        pick_bounds(&w, camera, Vec2::new(0.9, 0.9)).is_none(),
        "beside"
    );
}
