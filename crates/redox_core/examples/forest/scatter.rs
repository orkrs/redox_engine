//! Where things stand in the forest: a seeded scatter of trees, boulders, ferns and
//! grass over gently rolling ground.

use std::collections::HashMap;

use redox_math::Vec3;

/// Models the scene uses, in the order they are loaded.
pub const MODEL_NAMES: &[&str] = &[
    "fir_a",
    "fir_b",
    "fir_c",
    "fir_a_lod",
    "fir_b_lod",
    "fir_c_lod",
    "boulder",
    "boulder_mid",
    "boulder_lod",
    "fern_a",
    "fern_b",
    "fern_c",
    "fern_d",
    "grass2_a",
    "grass2_b",
    "grass2_c",
    "grass2_d",
    "grass2_e",
    "grass1_mid_a",
    "grass1_mid_b",
    "grass1_tall_a",
    "grass1_tall_b",
    "grass1_large_a",
    "grass1_small_a",
    "grass1_tiny_a",
];

/// One placed model.
pub struct Instance {
    /// Model index close to the camera.
    pub near: usize,
    /// Model index beyond `switch` metres.
    pub far: usize,
    pub switch: f32,
    /// Not drawn beyond this distance.
    pub cull: f32,
    pub pos: Vec3,
    pub yaw: f32,
    pub scale: f32,
}

pub struct World {
    pub instances: Vec<Instance>,
}

/// Rolling ground height (metres).
pub fn ground_height(x: f32, z: f32) -> f32 {
    0.55 * (x * 0.05).sin() * (z * 0.04).cos() + 0.25 * (x * 0.13 + z * 0.09).sin()
}

/// Small xorshift generator so the scene is the same on every run.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 40) as f32 / (1u64 << 24) as f32
    }

    fn range(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (hi - lo) * self.next()
    }

    fn pick<T: Copy>(&mut self, items: &[T]) -> T {
        items[((self.next() * items.len() as f32) as usize).min(items.len() - 1)]
    }
}

fn place(x: f32, z: f32, sink: f32) -> Vec3 {
    Vec3::new(x, ground_height(x, z) - sink, z)
}

/// One model at the origin, unscaled, for single-asset checks (`FOREST_ONLY`).
pub fn single() -> World {
    World {
        instances: vec![Instance {
            near: 0,
            far: 0,
            switch: f32::MAX,
            cull: f32::MAX,
            pos: Vec3::ZERO,
            yaw: 0.0,
            scale: 1.0,
        }],
    }
}

/// Builds the scene. `index` maps model names to their position in the loaded list.
pub fn populate(index: &HashMap<&str, usize>, seed: u64) -> World {
    let m = |name: &str| index[name];
    let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
    let mut instances = Vec::new();

    // Trees: rejection sampling keeps the trunks apart and the camera's glade open.
    let kinds = [
        (m("fir_a"), m("fir_a_lod")),
        (m("fir_b"), m("fir_b_lod")),
        (m("fir_c"), m("fir_c_lod")),
    ];
    let mut trunks: Vec<(f32, f32)> = Vec::new();
    let mut tries = 0;
    while trunks.len() < 230 && tries < 14000 {
        tries += 1;
        let (x, z) = (rng.range(-70.0, 70.0), rng.range(-110.0, 14.0));
        if (x * x + (z - 4.0) * (z - 4.0)).sqrt() < 5.0
            || trunks.iter().any(|t| (t.0 - x).hypot(t.1 - z) < 3.6)
        {
            continue;
        }
        trunks.push((x, z));
        let (near, far) = rng.pick(&kinds);
        instances.push(Instance {
            near,
            far,
            // The LOD1 branch cards of these trees draw as opaque planks; the LOD2 ones are right.
            switch: 0.0,
            cull: 220.0,
            pos: place(x, z, 0.15),
            yaw: rng.range(0.0, std::f32::consts::TAU),
            scale: rng.range(0.85, 1.2),
        });
    }

    // Boulders, some in the foreground at the right like in the reference picture.
    let rocks = [
        (6.0, -8.0, 3.4),
        (8.5, -3.0, 2.2),
        (-9.0, -14.0, 3.0),
        (-4.0, -24.0, 4.0),
        (14.0, -22.0, 3.6),
        (-18.0, -34.0, 4.2),
        (5.0, -40.0, 4.5),
        (-26.0, -12.0, 2.6),
        (24.0, -12.0, 3.0),
    ];
    for (x, z, scale) in rocks {
        instances.push(Instance {
            near: m("boulder"),
            far: m("boulder_mid"),
            switch: 45.0,
            cull: 180.0,
            pos: place(x, z, 0.35 * scale * 0.3),
            yaw: rng.range(0.0, std::f32::consts::TAU),
            scale,
        });
    }

    // Ferns.
    let ferns = [m("fern_a"), m("fern_b"), m("fern_c"), m("fern_d")];
    for _ in 0..260 {
        let (x, z) = (rng.range(-30.0, 30.0), rng.range(-50.0, 10.0));
        let model = rng.pick(&ferns);
        instances.push(Instance {
            near: model,
            far: model,
            switch: f32::MAX,
            cull: 55.0,
            pos: place(x, z, 0.0),
            yaw: rng.range(0.0, std::f32::consts::TAU),
            scale: rng.range(1.2, 2.4),
        });
    }

    // Grass: dense near the camera, thinner further out, mixed heights.
    let tall = [m("grass1_tall_a"), m("grass1_tall_b"), m("grass1_large_a")];
    let mid = [
        m("grass1_mid_a"),
        m("grass1_mid_b"),
        m("grass2_c"),
        m("grass2_d"),
        m("grass2_e"),
    ];
    let low = [
        m("grass1_small_a"),
        m("grass1_tiny_a"),
        m("grass2_a"),
        m("grass2_b"),
    ];
    for k in 0..18000 {
        let reach = if k < 13000 { 22.0 } else { 44.0 };
        let (x, z) = (
            rng.range(-reach, reach),
            rng.range(-reach * 1.4, reach * 0.35 + 6.0),
        );
        let model = match rng.next() {
            v if v < 0.35 => rng.pick(&tall),
            v if v < 0.75 => rng.pick(&mid),
            _ => rng.pick(&low),
        };
        instances.push(Instance {
            near: model,
            far: model,
            switch: f32::MAX,
            cull: 48.0,
            pos: place(x, z, 0.0),
            yaw: rng.range(0.0, std::f32::consts::TAU),
            scale: rng.range(2.0, 4.2),
        });
    }
    World { instances }
}
