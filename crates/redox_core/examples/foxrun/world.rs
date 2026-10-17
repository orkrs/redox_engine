//! The rules of Foxfire: a fox collects glowing gems in a forest glade before the lantern burns
//! out, sprinting costs stamina, and dark wisps take time away.
//!
//! Pure game logic (no rendering): the layout is generated from a seed, so tests can check the
//! rules exactly.

use std::f32::consts::{PI, TAU};

/// Radius of the circular glade, in metres.
pub const ARENA_RADIUS: f32 = 40.0;
/// Seconds on the clock at the start.
pub const START_TIME: f32 = 75.0;
/// Seconds a gem adds to the clock.
pub const GEM_TIME_BONUS: f32 = 2.5;
/// Seconds a wisp hit takes away.
pub const WISP_PENALTY: f32 = 4.0;
/// Walking speed at full stick, m/s.
pub const WALK_SPEED: f32 = 2.4;
/// Sprinting speed, m/s.
pub const RUN_SPEED: f32 = 6.2;
/// Radius of the fox's body for collisions.
pub const BODY_RADIUS: f32 = 0.45;
/// Distance at which a gem is picked up.
pub const PICKUP_RADIUS: f32 = 1.1;
/// Distance at which a wisp touches the fox.
pub const WISP_RADIUS: f32 = 0.75;
/// Gems in a run.
pub const GEM_COUNT: usize = 24;

const STAMINA_DRAIN: f32 = 0.38;
const STAMINA_REGEN: f32 = 0.22;
/// After running dry the fox cannot sprint until the stamina is back above this level.
const STAMINA_RESUME: f32 = 0.3;
const TURN_RATE: f32 = 9.0;
const ACCELERATION: f32 = 14.0;
const HIT_COOLDOWN: f32 = 1.4;
const KNOCKBACK: f32 = 2.2;
/// Seconds in which picking up the next gem keeps the combo alive.
const COMBO_WINDOW: f32 = 3.0;
const MAX_COMBO: u32 = 5;
const GEM_SCORE: u32 = 100;
const TIME_SCORE: f32 = 20.0;

/// A round thing the fox cannot walk through (tree trunk or rock).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Obstacle {
    pub x: f32,
    pub z: f32,
    pub radius: f32,
    /// Trees are tall with a leafy crown, rocks are low.
    pub tree: bool,
}

/// A collectible.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Gem {
    pub x: f32,
    pub z: f32,
    pub taken: bool,
}

/// A dark wisp that circles a point.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Wisp {
    pub center: (f32, f32),
    pub radius: f32,
    /// Angular speed in rad/s (sign = direction).
    pub speed: f32,
    pub phase: f32,
}

impl Wisp {
    /// Where the wisp is `t` seconds after the start.
    pub fn position(&self, t: f32) -> (f32, f32) {
        let a = self.phase + self.speed * t;
        (
            self.center.0 + a.cos() * self.radius,
            self.center.1 + a.sin() * self.radius,
        )
    }
}

/// How a run is going.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Playing,
    Won,
    Lost,
}

/// What the player asks for this frame.
#[derive(Clone, Copy, Debug, Default)]
pub struct Controls {
    /// Right is +1.
    pub move_x: f32,
    /// Away from the camera is +1.
    pub move_z: f32,
    /// Sprint button held.
    pub sprint: bool,
    /// Camera rotation, -1..1.
    pub camera_turn: f32,
}

/// What happened during one [`World::step`].
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Events {
    pub gems: usize,
    pub hit: bool,
    pub finished: Option<Phase>,
}

/// SplitMix64: small, seedable, good enough for placing trees.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> f32 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^= z >> 31;
        (z >> 40) as f32 / (1u64 << 24) as f32
    }

    fn range(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (hi - lo) * self.next()
    }
}

/// The glade and the run in progress.
pub struct World {
    pub obstacles: Vec<Obstacle>,
    pub gems: Vec<Gem>,
    pub wisps: Vec<Wisp>,
    pub pos: (f32, f32),
    pub yaw: f32,
    pub camera_yaw: f32,
    /// Current ground speed, m/s.
    pub speed: f32,
    pub stamina: f32,
    pub exhausted: bool,
    pub sprinting: bool,
    pub time_left: f32,
    /// Seconds since the start of the run.
    pub elapsed: f32,
    pub score: u32,
    pub combo: u32,
    combo_timer: f32,
    hit_cooldown: f32,
    pub phase: Phase,
}

impl World {
    /// A new run on the glade generated from `seed`.
    pub fn new(seed: u64) -> Self {
        let mut rng = Rng(seed);
        let mut obstacles: Vec<Obstacle> = Vec::new();
        let spawn = (0.0f32, -ARENA_RADIUS * 0.8);
        let mut tries = 0;
        while obstacles.len() < 42 && tries < 2000 {
            tries += 1;
            let tree = obstacles.len() < 30;
            let radius = if tree {
                rng.range(0.35, 0.55)
            } else {
                rng.range(0.9, 1.7)
            };
            let angle = rng.range(0.0, TAU);
            let distance = rng.range(5.0, ARENA_RADIUS - 2.0);
            let (x, z) = (angle.cos() * distance, angle.sin() * distance);
            let near_spawn = (x - spawn.0).hypot(z - spawn.1) < 6.0;
            let overlaps = obstacles
                .iter()
                .any(|o| (o.x - x).hypot(o.z - z) < o.radius + radius + 2.5);
            if !near_spawn && !overlaps {
                obstacles.push(Obstacle { x, z, radius, tree });
            }
        }

        let mut gems: Vec<Gem> = Vec::new();
        tries = 0;
        while gems.len() < GEM_COUNT && tries < 4000 {
            tries += 1;
            let angle = rng.range(0.0, TAU);
            let distance = rng.range(6.0, ARENA_RADIUS - 3.0);
            let (x, z) = (angle.cos() * distance, angle.sin() * distance);
            let blocked = obstacles
                .iter()
                .any(|o| (o.x - x).hypot(o.z - z) < o.radius + 1.6);
            let crowded = gems.iter().any(|g| (g.x - x).hypot(g.z - z) < 5.0);
            if !blocked && !crowded {
                gems.push(Gem { x, z, taken: false });
            }
        }

        let wisps = (0..5)
            .map(|i| {
                let angle = TAU * i as f32 / 5.0 + rng.range(-0.3, 0.3);
                let distance = rng.range(10.0, 28.0);
                Wisp {
                    center: (angle.cos() * distance, angle.sin() * distance),
                    radius: rng.range(3.0, 7.0),
                    speed: rng.range(0.5, 1.1) * if i % 2 == 0 { 1.0 } else { -1.0 },
                    phase: rng.range(0.0, TAU),
                }
            })
            .collect();

        Self {
            obstacles,
            gems,
            wisps,
            pos: spawn,
            yaw: PI / 2.0 * 0.0,
            camera_yaw: 0.0,
            speed: 0.0,
            stamina: 1.0,
            exhausted: false,
            sprinting: false,
            time_left: START_TIME,
            elapsed: 0.0,
            score: 0,
            combo: 0,
            combo_timer: 0.0,
            hit_cooldown: 0.0,
            phase: Phase::Playing,
        }
    }

    pub fn gems_taken(&self) -> usize {
        self.gems.iter().filter(|g| g.taken).count()
    }

    /// Camera forward direction on the ground (x, z).
    pub fn camera_forward(&self) -> (f32, f32) {
        (self.camera_yaw.sin(), self.camera_yaw.cos())
    }

    /// Whether the fox is currently untouchable after a hit.
    pub fn invulnerable(&self) -> bool {
        self.hit_cooldown > 0.0
    }

    /// Advances the run by `dt` seconds.
    pub fn step(&mut self, dt: f32, controls: Controls) -> Events {
        let mut events = Events::default();
        if self.phase != Phase::Playing || !dt.is_finite() || dt <= 0.0 {
            return events;
        }
        self.elapsed += dt;
        self.time_left -= dt;
        self.hit_cooldown = (self.hit_cooldown - dt).max(0.0);
        self.combo_timer = (self.combo_timer - dt).max(0.0);
        if self.combo_timer == 0.0 {
            self.combo = 0;
        }
        self.camera_yaw += controls.camera_turn.clamp(-1.0, 1.0) * 2.0 * dt;

        self.steer(dt, controls);
        self.collide();
        events.gems = self.pick_up_gems();
        events.hit = self.touch_wisps();

        if self.gems_taken() == self.gems.len() {
            self.score += (self.time_left.max(0.0) * TIME_SCORE) as u32;
            self.phase = Phase::Won;
            events.finished = Some(Phase::Won);
        } else if self.time_left <= 0.0 {
            self.time_left = 0.0;
            self.phase = Phase::Lost;
            events.finished = Some(Phase::Lost);
        }
        events
    }

    fn steer(&mut self, dt: f32, controls: Controls) {
        let (fx, fz) = self.camera_forward();
        let (rx, rz) = (-fz, fx);
        let wish_x = rx * controls.move_x + fx * controls.move_z;
        let wish_z = rz * controls.move_x + fz * controls.move_z;
        let strength = wish_x.hypot(wish_z).min(1.0);
        let moving = strength > 0.05;

        self.sprinting = controls.sprint && moving && !self.exhausted && self.stamina > 0.0;
        if self.sprinting {
            self.stamina = (self.stamina - STAMINA_DRAIN * dt).max(0.0);
            if self.stamina == 0.0 {
                self.exhausted = true;
            }
        } else {
            self.stamina = (self.stamina + STAMINA_REGEN * dt).min(1.0);
            if self.exhausted && self.stamina >= STAMINA_RESUME {
                self.exhausted = false;
            }
        }

        let target_speed = if !moving {
            0.0
        } else if self.sprinting {
            RUN_SPEED
        } else {
            WALK_SPEED * strength
        };
        let step = ACCELERATION * dt;
        self.speed += (target_speed - self.speed).clamp(-step, step);

        if moving {
            let target = wish_x.atan2(wish_z);
            let delta = (target - self.yaw + PI).rem_euclid(TAU) - PI;
            let max = TURN_RATE * dt;
            self.yaw += delta.clamp(-max, max);
        }
        self.pos.0 += self.yaw.sin() * self.speed * dt;
        self.pos.1 += self.yaw.cos() * self.speed * dt;
    }

    fn collide(&mut self) {
        let limit = ARENA_RADIUS - BODY_RADIUS;
        let distance = self.pos.0.hypot(self.pos.1);
        if distance > limit {
            let k = limit / distance;
            self.pos = (self.pos.0 * k, self.pos.1 * k);
        }
        for o in &self.obstacles {
            let reach = o.radius + BODY_RADIUS;
            let (dx, dz) = (self.pos.0 - o.x, self.pos.1 - o.z);
            let d = dx.hypot(dz);
            if d < reach {
                let (nx, nz) = if d > 1e-4 {
                    (dx / d, dz / d)
                } else {
                    (1.0, 0.0)
                };
                self.pos = (o.x + nx * reach, o.z + nz * reach);
            }
        }
    }

    fn pick_up_gems(&mut self) -> usize {
        let mut picked = 0;
        for gem in &mut self.gems {
            if !gem.taken && (self.pos.0 - gem.x).hypot(self.pos.1 - gem.z) < PICKUP_RADIUS {
                gem.taken = true;
                picked += 1;
            }
        }
        for _ in 0..picked {
            self.combo = (self.combo + 1).min(MAX_COMBO);
            self.combo_timer = COMBO_WINDOW;
            self.score += GEM_SCORE * self.combo;
            self.time_left += GEM_TIME_BONUS;
        }
        picked
    }

    fn touch_wisps(&mut self) -> bool {
        if self.hit_cooldown > 0.0 {
            return false;
        }
        for wisp in &self.wisps {
            let (wx, wz) = wisp.position(self.elapsed);
            let (dx, dz) = (self.pos.0 - wx, self.pos.1 - wz);
            if dx.hypot(dz) < WISP_RADIUS + BODY_RADIUS {
                let d = dx.hypot(dz).max(1e-4);
                self.pos.0 += dx / d * KNOCKBACK;
                self.pos.1 += dz / d * KNOCKBACK;
                self.collide();
                self.time_left -= WISP_PENALTY;
                self.combo = 0;
                self.hit_cooldown = HIT_COOLDOWN;
                return true;
            }
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn idle() -> Controls {
        Controls::default()
    }

    fn forward(sprint: bool) -> Controls {
        Controls {
            move_z: 1.0,
            sprint,
            ..Controls::default()
        }
    }

    /// A world without wisps and obstacles, for isolated rule checks.
    fn empty_world() -> World {
        let mut w = World::new(1);
        w.obstacles.clear();
        w.wisps.clear();
        w.gems = vec![Gem {
            x: 0.0,
            z: 0.0,
            taken: false,
        }];
        w.pos = (0.0, -10.0);
        w
    }

    #[test]
    fn the_layout_is_deterministic_and_complete() {
        let a = World::new(7);
        let b = World::new(7);
        assert_eq!(a.obstacles, b.obstacles);
        assert_eq!(a.gems, b.gems);
        assert_eq!(a.wisps, b.wisps);
        assert_ne!(a.gems, World::new(8).gems);
        assert_eq!(a.gems.len(), GEM_COUNT, "all gems found a place");
        assert!(a.obstacles.len() >= 30);
        assert_eq!(a.wisps.len(), 5);
    }

    #[test]
    fn gems_are_reachable_and_inside_the_glade() {
        for seed in 0..20 {
            let w = World::new(seed);
            for g in &w.gems {
                assert!(g.x.hypot(g.z) < ARENA_RADIUS - 1.0, "seed {seed}");
                for o in &w.obstacles {
                    assert!(
                        (o.x - g.x).hypot(o.z - g.z) > o.radius + BODY_RADIUS,
                        "seed {seed}: a gem inside an obstacle"
                    );
                }
            }
            let spawn_blocked = w
                .obstacles
                .iter()
                .any(|o| (o.x - w.pos.0).hypot(o.z - w.pos.1) < o.radius + 2.0);
            assert!(!spawn_blocked, "seed {seed}: the fox spawns inside a tree");
        }
    }

    #[test]
    fn standing_still_does_not_move_and_walking_does() {
        let mut w = empty_world();
        let start = w.pos;
        for _ in 0..30 {
            w.step(1.0 / 60.0, idle());
        }
        assert_eq!(w.pos, start);
        for _ in 0..60 {
            w.step(1.0 / 60.0, forward(false));
        }
        assert!(w.pos.1 > start.1 + 1.5, "walked forward: {:?}", w.pos);
        assert!((w.speed - WALK_SPEED).abs() < 0.05);
    }

    #[test]
    fn sprinting_is_faster_and_drains_stamina() {
        let mut walk = empty_world();
        let mut run = empty_world();
        for _ in 0..60 {
            walk.step(1.0 / 60.0, forward(false));
            run.step(1.0 / 60.0, forward(true));
        }
        assert!(run.pos.1 > walk.pos.1 + 2.0);
        assert!(run.sprinting && run.stamina < 0.7);
        assert_eq!(walk.stamina, 1.0);
    }

    #[test]
    fn an_empty_stamina_bar_locks_the_sprint_until_it_recovers() {
        let mut w = empty_world();
        w.pos = (0.0, -20.0);
        w.gems[0].x = 30.0;
        w.gems[0].z = 30.0;
        let mut seconds = 0.0;
        while !w.exhausted && seconds < 10.0 {
            w.step(1.0 / 60.0, forward(true));
            seconds += 1.0 / 60.0;
        }
        assert!(w.exhausted, "stamina ran out");
        w.step(1.0 / 60.0, forward(true));
        assert!(!w.sprinting, "cannot sprint while exhausted");
        for _ in 0..240 {
            w.step(1.0 / 60.0, forward(false));
        }
        assert!(!w.exhausted && w.stamina > STAMINA_RESUME);
        w.step(1.0 / 60.0, forward(true));
        assert!(w.sprinting);
    }

    #[test]
    fn obstacles_block_and_the_glade_has_a_wall() {
        let mut w = empty_world();
        w.obstacles.push(Obstacle {
            x: 0.0,
            z: -5.0,
            radius: 1.0,
            tree: true,
        });
        w.gems[0].taken = false;
        for _ in 0..240 {
            w.step(1.0 / 60.0, forward(true));
            let d = (w.pos.0).hypot(w.pos.1 + 5.0);
            assert!(d >= 1.0 + BODY_RADIUS - 1e-3, "inside the trunk: {d}");
        }
        let mut edge = empty_world();
        edge.pos = (ARENA_RADIUS - 1.0, 0.0);
        edge.yaw = PI / 2.0;
        for _ in 0..120 {
            edge.step(
                1.0 / 60.0,
                Controls {
                    move_x: 1.0,
                    sprint: true,
                    ..Controls::default()
                },
            );
        }
        assert!(edge.pos.0.hypot(edge.pos.1) <= ARENA_RADIUS - BODY_RADIUS + 1e-3);
    }

    #[test]
    fn a_gem_adds_time_and_score_and_the_combo_grows() {
        let mut w = empty_world();
        w.gems = vec![
            Gem {
                x: 0.0,
                z: 0.0,
                taken: false,
            },
            Gem {
                x: 0.8,
                z: 0.5,
                taken: false,
            },
            Gem {
                x: 40.0,
                z: 0.0,
                taken: false,
            },
        ];
        w.pos = (0.0, 0.0);
        let before = w.time_left;
        let events = w.step(1.0 / 60.0, idle());
        assert_eq!(events.gems, 2, "both gems within reach");
        assert_eq!(w.combo, 2);
        assert_eq!(w.score, 100 + 200);
        assert!((w.time_left - (before + 2.0 * GEM_TIME_BONUS - 1.0 / 60.0)).abs() < 1e-3);
        assert_eq!(w.phase, Phase::Playing, "one gem is still out there");
    }

    #[test]
    fn the_combo_expires() {
        let mut w = empty_world();
        w.gems = vec![
            Gem {
                x: 0.0,
                z: 0.0,
                taken: false,
            },
            Gem {
                x: 30.0,
                z: 30.0,
                taken: false,
            },
        ];
        w.pos = (0.0, 0.0);
        w.step(1.0 / 60.0, idle());
        assert_eq!(w.combo, 1);
        for _ in 0..(4 * 60) {
            w.step(1.0 / 60.0, idle());
        }
        assert_eq!(w.combo, 0);
    }

    #[test]
    fn collecting_every_gem_wins_with_a_time_bonus() {
        let mut w = empty_world();
        w.pos = (0.0, 0.0);
        let events = w.step(1.0 / 60.0, idle());
        assert_eq!(events.finished, Some(Phase::Won));
        assert_eq!(w.phase, Phase::Won);
        assert!(w.score > 100 + 1000, "time bonus added: {}", w.score);
        let score = w.score;
        w.step(1.0, idle());
        assert_eq!(w.score, score, "a finished run is frozen");
    }

    #[test]
    fn running_out_of_time_loses() {
        let mut w = empty_world();
        w.gems[0].x = 30.0;
        w.time_left = 0.5;
        let mut last = Events::default();
        for _ in 0..60 {
            let e = w.step(1.0 / 60.0, idle());
            if e.finished.is_some() {
                last = e;
            }
        }
        assert_eq!(last.finished, Some(Phase::Lost));
        assert_eq!(w.phase, Phase::Lost);
        assert_eq!(w.time_left, 0.0);
    }

    #[test]
    fn a_wisp_costs_time_once_and_then_gives_a_grace_period() {
        let mut w = empty_world();
        w.gems[0].x = 30.0;
        w.wisps = vec![Wisp {
            center: (0.0, -10.0),
            radius: 0.0,
            speed: 0.0,
            phase: 0.0,
        }];
        let before = w.time_left;
        let first = w.step(1.0 / 60.0, idle());
        assert!(first.hit);
        assert!(w.invulnerable());
        assert!((w.time_left - (before - WISP_PENALTY - 1.0 / 60.0)).abs() < 1e-3);
        // Knocked away: another touch right now is not possible anyway, and the grace period holds.
        w.pos = (0.0, -10.0);
        let second = w.step(1.0 / 60.0, idle());
        assert!(!second.hit, "grace period");
        w.pos = (10.0, 10.0);
        for _ in 0..120 {
            w.step(1.0 / 60.0, idle());
        }
        assert!(!w.invulnerable());
    }

    #[test]
    fn the_fox_turns_towards_where_it_goes() {
        let mut w = empty_world();
        w.step(
            0.5,
            Controls {
                move_x: 1.0,
                ..Controls::default()
            },
        );
        // The camera looks along +z, so screen-right is -x and the fox turns to yaw -90 degrees.
        assert!((w.yaw + PI / 2.0).abs() < 0.2, "faces right: {}", w.yaw);
    }

    #[test]
    fn bad_time_steps_change_nothing() {
        let mut w = empty_world();
        let (time, pos) = (w.time_left, w.pos);
        for dt in [0.0, -1.0, f32::NAN] {
            w.step(dt, forward(true));
        }
        assert_eq!((w.time_left, w.pos), (time, pos));
    }

    #[test]
    fn wisps_move_on_circles() {
        let wisp = Wisp {
            center: (1.0, 2.0),
            radius: 3.0,
            speed: 1.0,
            phase: 0.0,
        };
        let (x0, z0) = wisp.position(0.0);
        assert!((x0 - 4.0).abs() < 1e-5 && (z0 - 2.0).abs() < 1e-5);
        let (x, z) = wisp.position(1.7);
        assert!(((x - 1.0).hypot(z - 2.0) - 3.0).abs() < 1e-4);
    }
}
