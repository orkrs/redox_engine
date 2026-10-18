//! Game rules: level data, the character controller driven by root motion, coins and crates.
//! Nothing here touches the GPU, so it is covered by unit tests.

use redox_anim::graph::{GraphOutput, GraphPlayer};
use redox_anim::{AnimationClip, Skeleton};
use redox_math::{Quat, Vec3};

use crate::rig;

const TURN_RATE: f32 = 12.0;
const BODY_RADIUS: f32 = 0.3;
pub const PICKUP_RADIUS: f32 = 0.7;
/// How fast the upper-body layer fades in and out (per second).
const LAYER_FADE: f32 = 10.0;

/// A crate: an axis-aligned block on the ground.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Crate {
    pub x: f32,
    pub z: f32,
    pub half: f32,
}

/// Level data, read from `level.txt` in the pak.
#[derive(Clone, Debug, PartialEq)]
pub struct Level {
    /// Half the side of the square arena.
    pub half: f32,
    pub spawn: (f32, f32),
    pub coins: Vec<(f32, f32)>,
    pub crates: Vec<Crate>,
}

/// Parses `arena <half>`, `spawn x z`, `coin x z`, `crate x z half`; `#` starts a comment.
pub fn parse_level(text: &str) -> Result<Level, String> {
    let mut level = Level {
        half: 12.0,
        spawn: (0.0, 0.0),
        coins: Vec::new(),
        crates: Vec::new(),
    };
    for (number, line) in text.lines().enumerate() {
        let line = line.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        let mut words = line.split_whitespace();
        let command = words.next().unwrap_or("");
        let numbers: Result<Vec<f32>, _> = words.map(str::parse::<f32>).collect();
        let n = numbers.map_err(|e| format!("line {}: {e}", number + 1))?;
        let bad = |expected: &str| format!("line {}: `{command}` needs {expected}", number + 1);
        match (command, n.as_slice()) {
            ("arena", [half]) if *half > 1.0 => level.half = *half,
            ("arena", _) => return Err(bad("one size above 1")),
            ("spawn", [x, z]) => level.spawn = (*x, *z),
            ("spawn", _) => return Err(bad("x z")),
            ("coin", [x, z]) => level.coins.push((*x, *z)),
            ("coin", _) => return Err(bad("x z")),
            ("crate", [x, z, half]) if *half > 0.0 => level.crates.push(Crate {
                x: *x,
                z: *z,
                half: *half,
            }),
            ("crate", _) => return Err(bad("x z half")),
            _ => return Err(format!("line {}: unknown command `{command}`", number + 1)),
        }
    }
    if level.coins.is_empty() {
        return Err("a level needs at least one coin".into());
    }
    Ok(level)
}

/// What the player asks for this frame.
#[derive(Clone, Copy, Debug, Default)]
pub struct Controls {
    /// Stick / keys, right is +x.
    pub move_x: f32,
    /// Stick / keys, forward (away from the camera) is +z.
    pub move_z: f32,
    /// Start the wave (one press).
    pub wave: bool,
    /// Camera rotation, -1..1.
    pub camera_turn: f32,
}

/// The character and the world around it.
pub struct World {
    pub level: Level,
    pub taken: Vec<bool>,
    pub pos: Vec3,
    pub yaw: f32,
    pub camera_yaw: f32,
    pub elapsed: f32,
    /// Time at which the last coin was taken.
    pub finished: Option<f32>,
    graph: GraphPlayer,
    wave_weight: f32,
}

/// Result of one [`World::step`].
pub struct Step {
    pub output: GraphOutput,
    pub picked: usize,
}

impl World {
    pub fn new(level: Level, skeleton: &Skeleton) -> Self {
        let pos = Vec3::new(level.spawn.0, 0.0, level.spawn.1);
        Self {
            taken: vec![false; level.coins.len()],
            level,
            pos,
            yaw: 0.0,
            camera_yaw: 0.0,
            elapsed: 0.0,
            finished: None,
            graph: GraphPlayer::new(rig::graph(), skeleton),
            wave_weight: 0.0,
        }
    }

    pub fn coins_taken(&self) -> usize {
        self.taken.iter().filter(|t| **t).count()
    }

    /// Name of the base-layer animation state (for the HUD).
    pub fn animation_state(&self) -> &str {
        self.graph.state_name(0).unwrap_or("-")
    }

    /// Whether the wave is playing.
    pub fn waving(&self) -> bool {
        self.graph.state_name(1) == Some("wave")
    }

    pub fn camera_forward(&self) -> Vec3 {
        Vec3::new(self.camera_yaw.sin(), 0.0, self.camera_yaw.cos())
    }

    /// Advances the character by `dt` and returns the pose to draw.
    pub fn step(
        &mut self,
        dt: f32,
        controls: Controls,
        skeleton: &Skeleton,
        clips: &[AnimationClip],
    ) -> Step {
        self.elapsed += dt;
        self.camera_yaw += controls.camera_turn.clamp(-1.0, 1.0) * 2.0 * dt;

        let forward = self.camera_forward();
        let right = Vec3::new(-forward.z, 0.0, forward.x);
        let wish = right * controls.move_x + forward * controls.move_z;
        let strength = wish.length().min(1.0);
        if strength > 0.05 {
            let target = wish.x.atan2(wish.z);
            let delta = (target - self.yaw + std::f32::consts::PI)
                .rem_euclid(std::f32::consts::TAU)
                - std::f32::consts::PI;
            let max = TURN_RATE * dt;
            self.yaw += delta.clamp(-max, max);
        }
        self.graph
            .params
            .set_float("speed", if strength > 0.05 { strength } else { 0.0 });
        if controls.wave {
            self.graph.params.trigger("wave");
        }
        let output = self.graph.update(dt, skeleton, clips);

        // Fade the arm layer in while the wave plays.
        let target = if self.waving() { 1.0 } else { 0.0 };
        let step = (LAYER_FADE * dt).min(1.0);
        self.wave_weight += (target - self.wave_weight) * step;
        self.graph.set_layer_weight(1, self.wave_weight);

        let world_delta = Quat::from_rotation_y(self.yaw) * output.root_motion.translation;
        self.pos = self.collide(self.pos + world_delta);
        let picked = self.pick_up_coins();
        if self.finished.is_none() && self.coins_taken() == self.taken.len() {
            self.finished = Some(self.elapsed);
        }
        Step { output, picked }
    }

    fn collide(&self, mut pos: Vec3) -> Vec3 {
        let limit = self.level.half - 0.5;
        pos.x = pos.x.clamp(-limit, limit);
        pos.z = pos.z.clamp(-limit, limit);
        for c in &self.level.crates {
            let reach = c.half + BODY_RADIUS;
            let (dx, dz) = (pos.x - c.x, pos.z - c.z);
            if dx.abs() < reach && dz.abs() < reach {
                // Push out along the axis with the smaller overlap.
                if reach - dx.abs() < reach - dz.abs() {
                    pos.x = c.x + reach * dx.signum();
                } else {
                    pos.z = c.z + reach * dz.signum();
                }
            }
        }
        pos
    }

    fn pick_up_coins(&mut self) -> usize {
        let mut picked = 0;
        for (taken, (x, z)) in self.taken.iter_mut().zip(&self.level.coins) {
            if !*taken && (self.pos.x - x).hypot(self.pos.z - z) < PICKUP_RADIUS {
                *taken = true;
                picked += 1;
            }
        }
        picked
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LEVEL: &str = "# test\narena 10\nspawn 0 0\ncoin 0 3\ncoin 5 5\ncrate 0 1.5 0.5\n";

    fn new_world() -> (World, redox_anim::SkinnedModel) {
        let model = rig::humanoid();
        let level = parse_level(LEVEL).unwrap();
        (World::new(level, &model.skeleton), model)
    }

    fn run(world: &mut World, model: &redox_anim::SkinnedModel, seconds: f32, controls: Controls) {
        let frames = (seconds / 0.016) as usize;
        for _ in 0..frames {
            world.step(0.016, controls, &model.skeleton, &model.clips);
        }
    }

    const FORWARD: Controls = Controls {
        move_x: 0.0,
        move_z: 1.0,
        wave: false,
        camera_turn: 0.0,
    };

    #[test]
    fn levels_parse_and_report_the_line_of_a_mistake() {
        let level = parse_level(LEVEL).unwrap();
        assert_eq!(level.half, 10.0);
        assert_eq!(level.coins.len(), 2);
        assert_eq!(
            level.crates,
            vec![Crate {
                x: 0.0,
                z: 1.5,
                half: 0.5
            }]
        );
        assert!(parse_level("coin 1").unwrap_err().contains("line 1"));
        assert!(
            parse_level("coin 1 2\nfly 3")
                .unwrap_err()
                .contains("line 2")
        );
        assert!(parse_level("coin a b").unwrap_err().contains("line 1"));
        assert!(parse_level("arena 10").unwrap_err().contains("coin"));
    }

    #[test]
    fn standing_still_stays_idle_and_does_not_move() {
        let (mut world, model) = new_world();
        run(&mut world, &model, 1.0, Controls::default());
        assert_eq!(world.animation_state(), "idle");
        assert!(world.pos.length() < 1e-4);
    }

    #[test]
    fn full_stick_runs_at_the_clip_speed_through_root_motion() {
        let (mut world, model) = new_world();
        // Aim away from the crate (it is straight ahead at z = 1.5).
        world.yaw = std::f32::consts::FRAC_PI_2;
        let controls = Controls {
            move_x: 0.0,
            move_z: 1.0,
            ..Default::default()
        };
        world.camera_yaw = std::f32::consts::FRAC_PI_2;
        run(&mut world, &model, 0.4, controls);
        let before = world.pos;
        run(&mut world, &model, 1.0, controls);
        let speed = (world.pos - before).length();
        assert_eq!(world.animation_state(), "move");
        assert!((speed - rig::RUN_SPEED).abs() < 0.4, "ran {speed} m in 1 s");
    }

    #[test]
    fn half_stick_moves_slower_than_full_stick() {
        let (mut slow, model) = new_world();
        let (mut fast, _) = new_world();
        for w in [&mut slow, &mut fast] {
            w.camera_yaw = std::f32::consts::FRAC_PI_2;
            w.yaw = std::f32::consts::FRAC_PI_2;
        }
        run(
            &mut slow,
            &model,
            1.5,
            Controls {
                move_z: 0.4,
                ..Default::default()
            },
        );
        run(
            &mut fast,
            &model,
            1.5,
            Controls {
                move_z: 1.0,
                ..Default::default()
            },
        );
        assert!(fast.pos.length() > slow.pos.length() * 1.5);
    }

    #[test]
    fn the_character_turns_towards_where_it_goes() {
        let (mut world, model) = new_world();
        run(
            &mut world,
            &model,
            0.5,
            Controls {
                move_x: 1.0,
                ..Default::default()
            },
        );
        // Camera looks along +z, so screen-right is -x: the character should face -x.
        let facing = Vec3::new(world.yaw.sin(), 0.0, world.yaw.cos());
        assert!(facing.x < -0.9, "{facing:?}");
    }

    #[test]
    fn crates_block_and_the_arena_has_walls() {
        let (mut world, model) = new_world();
        run(&mut world, &model, 2.0, FORWARD);
        assert!(
            world.pos.z < 1.5 - 0.5 - BODY_RADIUS + 0.05,
            "stopped at the crate: {}",
            world.pos.z
        );
        let (mut world, model) = new_world();
        world.yaw = std::f32::consts::FRAC_PI_2;
        world.camera_yaw = std::f32::consts::FRAC_PI_2;
        run(&mut world, &model, 6.0, FORWARD);
        assert!(
            (world.pos.x - 9.5).abs() < 1e-3,
            "stopped at the wall: {}",
            world.pos.x
        );
    }

    #[test]
    fn coins_are_collected_once_and_finish_the_level() {
        let (mut world, model) = new_world();
        world.pos = Vec3::new(0.0, 0.0, 3.0);
        let step = world.step(0.016, Controls::default(), &model.skeleton, &model.clips);
        assert_eq!(step.picked, 1);
        let again = world.step(0.016, Controls::default(), &model.skeleton, &model.clips);
        assert_eq!(again.picked, 0, "a coin counts once");
        assert!(world.finished.is_none());
        world.pos = Vec3::new(5.0, 0.0, 5.0);
        world.step(0.016, Controls::default(), &model.skeleton, &model.clips);
        assert_eq!(world.coins_taken(), 2);
        assert!(world.finished.is_some());
    }

    #[test]
    fn waving_plays_on_the_arm_layer_and_ends_by_itself() {
        let (mut world, model) = new_world();
        world.step(
            0.016,
            Controls {
                wave: true,
                ..Default::default()
            },
            &model.skeleton,
            &model.clips,
        );
        run(&mut world, &model, 0.3, Controls::default());
        assert!(world.waving());
        let out = world.step(0.016, Controls::default(), &model.skeleton, &model.clips);
        let arm = model.skeleton.bone_index("upper_arm_r").unwrap();
        let raised = out.output.pose.bones[arm]
            .rotation
            .angle_between(Quat::IDENTITY);
        assert!(raised > 1.0, "arm is up: {raised}");
        run(&mut world, &model, 1.5, Controls::default());
        assert!(!world.waving(), "the one-shot returned to rest");
    }
}
