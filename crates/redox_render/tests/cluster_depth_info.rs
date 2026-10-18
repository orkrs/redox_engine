//! The depth slices used by the fragment shader (`cluster_info` uniform) must follow the camera's
//! near and far planes, exactly like the slice bounds used to cull lights. When they diverge, a
//! point light is assigned to the wrong depth slices and its glow is cut by flat planes that move
//! with the camera. Skipped without a GPU.

#![allow(clippy::unwrap_used)]

mod common;

use redox_render::camera::Camera;
use redox_render::clustering::cluster_config::CLUSTER_DEPTH;

#[test]
fn the_cluster_info_follows_the_cameras_near_and_far() {
    let Some(adapter) = common::adapter() else {
        eprintln!("no GPU adapter, skipping");
        return;
    };
    let Some(mut renderer) = common::renderer(&adapter, 64) else {
        return;
    };
    for (near, far) in [(0.1, 160.0), (0.3, 40.0), (0.05, 500.0)] {
        let camera = Camera::new(0.9, 1.0, near, far);
        renderer.update_cluster_lights(&[], &camera);
        let info = renderer.cluster_manager().info();
        assert_eq!((info.near, info.far), (near, far));
        let expected = (far / near).ln() / (CLUSTER_DEPTH - 1) as f32;
        assert!(
            (info.depth_scale - expected).abs() < 1e-5,
            "depth_scale {} vs {expected} for near {near}, far {far}",
            info.depth_scale
        );
    }
}
