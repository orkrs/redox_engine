//! Visual node canvases: StateTree debugger, AnimGraph, PCG placeholder.

use redox_anim::{Inertializer, LocalPose, Pose, SkinnedModel};
use redox_ecs::Entity;
use redox_gameplay::{AttributeSet, StateTree, StateTreeComponent, TransitionCondition};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum GraphMode {
    #[default]
    StateTree,
    AnimGraph,
    Pcg,
}

#[derive(Clone, Debug)]
pub struct Node {
    pub id: usize,
    pub kind: String,
    pub pos: egui::Pos2,
}

#[derive(Clone, Debug)]
pub struct Edge {
    pub from: usize,
    pub to: usize,
    pub label: String,
}

#[derive(Clone, Debug, Default)]
pub struct NodeGraph {
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
}

/// Pixels per world unit in the AnimGraph skeleton preview.
const PREVIEW_SCALE: f32 = 56.0;

pub struct NodeCanvas {
    pub mode: GraphMode,
    pub graph: NodeGraph,
    pub active_id: Option<usize>,
    pub distance_preview: f32,
    pub anim_time: f32,
    pub anim_playing: bool,
    /// Rig and clips driving the AnimGraph (procedural demo rig until a model is loaded).
    pub rig: SkinnedModel,
    pub active_clip: usize,
    /// BlendSpace1D parameter (0 = first clip, 1 = second).
    pub blend_weight: f32,
    /// Inertialization between the poses before and after a clip switch.
    pub inertializer: Inertializer,
    /// Last entity whose StateTree was loaded into the canvas (avoids resetting node positions).
    pub synced_agent: Option<Entity>,
}

impl Default for NodeCanvas {
    fn default() -> Self {
        let mut s = Self {
            mode: GraphMode::StateTree,
            graph: NodeGraph::default(),
            active_id: None,
            distance_preview: 10.0,
            anim_time: 0.0,
            anim_playing: false,
            rig: redox_anim::procedural::demo_rig(),
            active_clip: 0,
            blend_weight: 0.35,
            inertializer: Inertializer::new(0.15),
            synced_agent: None,
        };
        s.load_patrol_chase();
        s
    }
}

impl NodeCanvas {
    pub fn load_patrol_chase(&mut self) {
        let tree = redox_gameplay::sentinel_patrol_chase();
        self.graph_from_state_tree(&tree);
        self.active_id = Some(tree.active_id() as usize);
        self.mode = GraphMode::StateTree;
    }

    pub fn sync_from_agent(&mut self, agent: &StateTreeComponent) {
        self.graph_from_state_tree(&agent.tree);
        self.active_id = Some(agent.tree.active_id() as usize);
    }

    /// Rebuild graph topology when the source entity changes; otherwise only refresh active state.
    pub fn sync_agent_selection(
        &mut self,
        entity: Option<Entity>,
        agent: Option<&StateTreeComponent>,
    ) {
        let Some(a) = agent else {
            return;
        };
        if self.synced_agent != entity {
            self.sync_from_agent(a);
            self.synced_agent = entity;
        } else {
            self.active_id = Some(a.tree.active_id() as usize);
        }
    }

    pub fn graph_from_state_tree(&mut self, tree: &StateTree) {
        let mut sorted: Vec<_> = tree.iter_nodes().cloned().collect();
        sorted.sort_by_key(|n| n.id);

        let mut nodes = Vec::with_capacity(sorted.len());
        let mut id_to_idx = std::collections::HashMap::new();
        for (i, n) in sorted.iter().enumerate() {
            id_to_idx.insert(n.id, i);
            let col = (i % 3) as f32;
            let row = (i / 3) as f32;
            nodes.push(Node {
                id: n.id as usize,
                kind: n.name.clone(),
                pos: egui::pos2(80.0 + col * 180.0, 60.0 + row * 100.0),
            });
        }

        let mut edges = Vec::new();
        for n in &sorted {
            let Some(&from) = id_to_idx.get(&n.id) else {
                continue;
            };
            for t in &n.transitions {
                if let Some(&to) = id_to_idx.get(&t.target) {
                    edges.push(Edge {
                        from,
                        to,
                        label: cond_label(&t.condition),
                    });
                }
            }
            if let Some(p) = n.parent {
                if let Some(&to) = id_to_idx.get(&p) {
                    edges.push(Edge {
                        from,
                        to,
                        label: "parent".into(),
                    });
                }
            }
        }
        self.graph = NodeGraph { nodes, edges };
    }

    pub fn load_anim_demo(&mut self) {
        self.mode = GraphMode::AnimGraph;
        self.active_clip = self.active_clip.min(self.rig.clips.len().saturating_sub(1));
        let (clip_a, clip_b) = self.blend_clip_names();
        self.graph = NodeGraph {
            nodes: vec![
                Node {
                    id: 0,
                    kind: format!("Clip:{clip_a}"),
                    pos: egui::pos2(60.0, 80.0),
                },
                Node {
                    id: 1,
                    kind: format!("Clip:{clip_b}"),
                    pos: egui::pos2(60.0, 160.0),
                },
                Node {
                    id: 2,
                    kind: "BlendSpace1D".into(),
                    pos: egui::pos2(260.0, 120.0),
                },
                Node {
                    id: 3,
                    kind: "StateMachine".into(),
                    pos: egui::pos2(460.0, 120.0),
                },
                Node {
                    id: 4,
                    kind: "Inertialization".into(),
                    pos: egui::pos2(660.0, 120.0),
                },
            ],
            edges: vec![
                Edge {
                    from: 0,
                    to: 2,
                    label: "0".into(),
                },
                Edge {
                    from: 1,
                    to: 2,
                    label: "1".into(),
                },
                Edge {
                    from: 2,
                    to: 3,
                    label: "speed".into(),
                },
                Edge {
                    from: 3,
                    to: 4,
                    label: "out".into(),
                },
            ],
        };
        self.active_id = Some(3);
    }

    pub fn load_pcg_demo(&mut self) {
        self.mode = GraphMode::Pcg;
        self.graph = NodeGraph {
            nodes: vec![
                Node {
                    id: 0,
                    kind: "GridBiome".into(),
                    pos: egui::pos2(80.0, 100.0),
                },
                Node {
                    id: 1,
                    kind: "PerlinNoise".into(),
                    pos: egui::pos2(260.0, 100.0),
                },
                Node {
                    id: 2,
                    kind: "PropScatter".into(),
                    pos: egui::pos2(440.0, 100.0),
                },
                Node {
                    id: 3,
                    kind: "OFPA_Exporter".into(),
                    pos: egui::pos2(620.0, 100.0),
                },
            ],
            edges: vec![
                Edge {
                    from: 0,
                    to: 1,
                    label: String::new(),
                },
                Edge {
                    from: 1,
                    to: 2,
                    label: String::new(),
                },
                Edge {
                    from: 2,
                    to: 3,
                    label: "bake".into(),
                },
            ],
        };
        self.active_id = None;
    }

    /// Names of the two clips the blend node mixes: the active clip and the next one.
    fn blend_clip_names(&self) -> (String, String) {
        let clips = &self.rig.clips;
        let name = |i: usize| clips.get(i).map_or_else(|| "-".to_string(), |c| c.name.clone());
        (name(self.active_clip), name((self.active_clip + 1) % clips.len().max(1)))
    }

    /// Local pose of the blend node at the current time: active clip mixed with the next clip.
    fn blended_pose(&self) -> LocalPose {
        let skeleton = &self.rig.skeleton;
        let clips = &self.rig.clips;
        let sample = |i: usize| {
            clips
                .get(i)
                .map_or_else(|| skeleton.bind_local_pose(), |c| c.sample(skeleton, self.anim_time, true))
        };
        let next = (self.active_clip + 1) % clips.len().max(1);
        sample(self.active_clip).blend(&sample(next), self.blend_weight)
    }

    /// Model-space pose including the inertialization offset of a recent clip switch.
    fn preview_pose(&self, dt: f32) -> Pose {
        let mut pose = Pose::from_local(&self.rig.skeleton, &self.blended_pose());
        self.inertializer.apply_damping(&mut pose, dt);
        pose
    }

    fn set_active_clip(&mut self, idx: usize, dt: f32) {
        if idx >= self.rig.clips.len() || idx == self.active_clip {
            return;
        }
        let old = Pose::from_local(&self.rig.skeleton, &self.blended_pose());
        self.active_clip = idx;
        self.anim_time = 0.0;
        let new = Pose::from_local(&self.rig.skeleton, &self.blended_pose());
        self.inertializer.record_transition(&old, &new, dt.max(1.0 / 60.0));
        self.active_id = Some(4);
    }

    /// Keeps the clip node labels in sync with the active clip.
    fn refresh_anim_labels(&mut self) {
        let (a, b) = self.blend_clip_names();
        for (id, name) in [(0, a), (1, b)] {
            if let Some(node) = self.graph.nodes.iter_mut().find(|n| n.id == id) {
                node.kind = format!("Clip:{name}");
            }
        }
    }

    /// Front-view (x/y) stick figure of the evaluated skeleton in the corner of the canvas.
    fn draw_skeleton_preview(&self, painter: &egui::Painter, canvas: egui::Rect, dt: f32) {
        let pose = self.preview_pose(dt);
        let origin = egui::pos2(canvas.right() - 90.0, canvas.bottom() - 18.0);
        let to_screen = |m: &redox_math::Mat4| {
            let p = m.w_axis;
            egui::pos2(origin.x + p.x * PREVIEW_SCALE, origin.y - p.y * PREVIEW_SCALE)
        };
        let frame = egui::Rect::from_min_max(
            egui::pos2(canvas.right() - 180.0, canvas.bottom() - 190.0),
            egui::pos2(canvas.right() - 6.0, canvas.bottom() - 6.0),
        );
        painter.rect_stroke(frame, 4.0, egui::Stroke::new(1.0, egui::Color32::from_gray(70)));
        painter.text(
            frame.left_top() + egui::vec2(6.0, 4.0),
            egui::Align2::LEFT_TOP,
            "pose preview",
            egui::FontId::monospace(10.0),
            egui::Color32::from_gray(150),
        );
        let points: Vec<egui::Pos2> = pose.bone_transforms.iter().map(to_screen).collect();
        for (i, bone) in self.rig.skeleton.bones.iter().enumerate() {
            if let Some(parent) = bone.parent.and_then(|p| points.get(p)) {
                painter.line_segment(
                    [*parent, points[i]],
                    egui::Stroke::new(3.0, egui::Color32::from_rgb(120, 220, 255)),
                );
            }
        }
        for p in &points {
            painter.circle_filled(*p, 3.5, egui::Color32::from_rgb(255, 200, 80));
        }
    }

    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        agent_entity: Option<Entity>,
        agent: Option<&mut StateTreeComponent>,
        attributes: Option<&AttributeSet>,
        skinned_time: Option<&mut f32>,
        dt: f32,
    ) {
        ui.horizontal(|ui| {
            if ui
                .selectable_label(self.mode == GraphMode::StateTree, "StateTree")
                .clicked()
            {
                self.synced_agent = None;
                self.load_patrol_chase();
            }
            if ui
                .selectable_label(self.mode == GraphMode::AnimGraph, "AnimGraph")
                .clicked()
            {
                self.load_anim_demo();
            }
            if ui
                .selectable_label(self.mode == GraphMode::Pcg, "PCG")
                .clicked()
            {
                self.load_pcg_demo();
            }
        });
        ui.separator();

        match self.mode {
            GraphMode::StateTree => {
                let mut do_tick = false;
                ui.horizontal(|ui| {
                    ui.label("distance_to_target");
                    ui.add(egui::Slider::new(&mut self.distance_preview, 0.0..=20.0));
                    do_tick = ui.button("Tick").clicked();
                    if let Some(a) = agent.as_ref() {
                        ui.colored_label(
                            egui::Color32::from_rgb(80, 255, 140),
                            format!("active: {}", a.tree.active_name()),
                        );
                        ui.label(format!("elapsed {:.2}s", a.ctx.elapsed_in_state));
                        if let Some(e) = agent_entity {
                            ui.label(format!("#{}", e.id()));
                        }
                    } else {
                        ui.label("(no agent — select entity with StateTree or spawn demo)");
                    }
                });
                if do_tick {
                    if let Some(a) = agent {
                        a.tick_with_distance(self.distance_preview, dt.max(1.0 / 60.0));
                        self.sync_from_agent(a);
                        self.synced_agent = agent_entity;
                    }
                } else {
                    self.sync_agent_selection(agent_entity, agent.as_deref());
                }
                if let Some(attrs) = attributes {
                    ui.label(format!(
                        "GAS  HP {:.0}/{:.0}  STA {:.0}/{:.0}",
                        attrs.health.current,
                        attrs.health.max,
                        attrs.stamina.current,
                        attrs.stamina.max
                    ));
                }
            }
            GraphMode::AnimGraph => {
                ui.horizontal(|ui| {
                    if ui
                        .button(if self.anim_playing { "Pause" } else { "Play" })
                        .clicked()
                    {
                        self.anim_playing = !self.anim_playing;
                    }
                    let clip_dur = self
                        .rig
                        .clips
                        .get(self.active_clip)
                        .map(|c| c.duration.max(0.1))
                        .unwrap_or(2.0);
                    ui.add(
                        egui::Slider::new(&mut self.anim_time, 0.0..=clip_dur).text("time"),
                    );
                    if self.anim_playing {
                        self.anim_time = (self.anim_time + dt) % clip_dur;
                    }
                    if let Some(t) = skinned_time {
                        *t = self.anim_time;
                        ui.label("(SkinnedMesh.time)");
                    }
                });
                ui.horizontal(|ui| {
                    ui.label("blend");
                    if ui
                        .add(
                            egui::Slider::new(&mut self.blend_weight, 0.0..=1.0)
                                .text("weight")
                                .fixed_decimals(2),
                        )
                        .changed()
                    {
                        self.active_id = Some(2);
                    }
                });
                ui.horizontal(|ui| {
                    ui.label("active clip");
                    let mut picked = self.active_clip;
                    for (i, clip) in self.rig.clips.iter().enumerate() {
                        if ui
                            .selectable_label(
                                picked == i,
                                format!(
                                    "{} ({:.1}s, {} keys)",
                                    clip.name,
                                    clip.duration,
                                    clip.keyframe_count()
                                ),
                            )
                            .clicked()
                        {
                            picked = i;
                        }
                    }
                    if picked != self.active_clip {
                        self.set_active_clip(picked, dt);
                    }
                });
                self.refresh_anim_labels();
                self.inertializer.tick(dt);
                let inert_status = if self.inertializer.is_active() {
                    format!(
                        "Inertialization ACTIVE  t={:.2}/{:.2}s  bones={}",
                        self.inertializer.elapsed,
                        self.inertializer.duration,
                        self.inertializer.diffs.len()
                    )
                } else {
                    format!(
                        "Inertialization idle  duration={:.2}s",
                        self.inertializer.duration
                    )
                };
                ui.horizontal(|ui| {
                    let col = if self.inertializer.is_active() {
                        egui::Color32::from_rgb(255, 200, 80)
                    } else {
                        egui::Color32::from_gray(160)
                    };
                    ui.colored_label(col, inert_status);
                    if self.inertializer.is_active() {
                        self.active_id = Some(4);
                    }
                });
            }
            GraphMode::Pcg => {
                ui.label("PCG graph → Packager tab / Build menu for OFPA bake");
            }
        }

        let (rect, response) = ui.allocate_exact_size(ui.available_size(), egui::Sense::drag());
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 4.0, egui::Color32::from_gray(28));

        if response.dragged() {
            if let Some(pos) = response.interact_pointer_pos() {
                let rel = pos - rect.min.to_vec2();
                if let Some(n) = self.graph.nodes.iter_mut().find(|n| {
                    egui::Rect::from_center_size(n.pos, egui::vec2(130.0, 44.0)).contains(rel)
                }) {
                    n.pos += response.drag_delta();
                }
            }
        }

        for edge in &self.graph.edges {
            if let (Some(a), Some(b)) = (
                self.graph.nodes.get(edge.from),
                self.graph.nodes.get(edge.to),
            ) {
                let p0 = a.pos + rect.min.to_vec2();
                let p1 = b.pos + rect.min.to_vec2();
                let col = if edge.label == "parent" {
                    egui::Color32::from_gray(80)
                } else {
                    egui::Color32::from_rgb(100, 180, 255)
                };
                painter.line_segment([p0, p1], egui::Stroke::new(2.0, col));
                if !edge.label.is_empty() && edge.label != "parent" {
                    painter.text(
                        egui::pos2((p0.x + p1.x) * 0.5, (p0.y + p1.y) * 0.5 - 8.0),
                        egui::Align2::CENTER_CENTER,
                        &edge.label,
                        egui::FontId::monospace(10.0),
                        egui::Color32::from_gray(180),
                    );
                }
            }
        }

        for node in &self.graph.nodes {
            let r = egui::Rect::from_center_size(
                node.pos + rect.min.to_vec2(),
                egui::vec2(130.0, 44.0),
            );
            let active = self.active_id == Some(node.id);
            let fill = if active {
                egui::Color32::from_rgb(20, 120, 70)
            } else {
                egui::Color32::from_gray(48)
            };
            painter.rect_filled(r, 6.0, fill);
            if active {
                painter.rect_stroke(
                    r,
                    6.0,
                    egui::Stroke::new(2.0, egui::Color32::from_rgb(80, 255, 140)),
                );
            }
            let label = if self.mode == GraphMode::AnimGraph && node.id == 2 {
                format!("Blend {:.0}%", self.blend_weight * 100.0)
            } else {
                node.kind.clone()
            };
            painter.text(
                r.center(),
                egui::Align2::CENTER_CENTER,
                label,
                egui::FontId::proportional(13.0),
                egui::Color32::WHITE,
            );
        }

        if self.mode == GraphMode::AnimGraph {
            self.draw_skeleton_preview(&painter, rect, dt);
        }
    }
}

fn cond_label(c: &TransitionCondition) -> String {
    match c {
        TransitionCondition::Always => "always".into(),
        TransitionCondition::Never => "never".into(),
        TransitionCondition::TargetCloserThan(t) => format!("dist≤{t}"),
        TransitionCondition::TargetFartherThan(t) => format!("dist≥{t}"),
        TransitionCondition::ElapsedGreaterThan(t) => format!("t≥{t}"),
        TransitionCondition::FlagSet(n) => format!("flag:{n}"),
        TransitionCondition::FlagClear(n) => format!("!{n}"),
        TransitionCondition::FloatAtLeast(n, v) => format!("{n}≥{v}"),
        TransitionCondition::FloatAtMost(n, v) => format!("{n}≤{v}"),
        TransitionCondition::And(_, _) => "and".into(),
        TransitionCondition::Or(_, _) => "or".into(),
        TransitionCondition::Not(_) => "not".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_canvas_uses_real_keyframed_clips() {
        let canvas = NodeCanvas::default();
        assert!(canvas.rig.clips.len() >= 2);
        assert!(canvas.rig.clips.iter().all(|c| c.keyframe_count() > 0));
    }

    /// slerp / lerp at t = 0 or 1 equal the endpoint only up to rounding.
    fn assert_same_pose(actual: &LocalPose, expected: &LocalPose) {
        assert_eq!(actual.bones.len(), expected.bones.len());
        for (a, e) in actual.bones.iter().zip(&expected.bones) {
            assert!(a.rotation.abs_diff_eq(e.rotation, 1.0e-5), "{:?} vs {:?}", a.rotation, e.rotation);
            assert!(a.translation.abs_diff_eq(e.translation, 1.0e-5));
        }
    }

    #[test]
    fn blended_pose_follows_blend_weight() {
        let mut canvas = NodeCanvas::default();
        canvas.anim_time = 0.4;
        canvas.blend_weight = 0.0;
        let a = canvas.rig.clips[0].sample(&canvas.rig.skeleton, 0.4, true);
        assert_same_pose(&canvas.blended_pose(), &a);
        canvas.blend_weight = 1.0;
        let b = canvas.rig.clips[1].sample(&canvas.rig.skeleton, 0.4, true);
        assert_same_pose(&canvas.blended_pose(), &b);
    }

    #[test]
    fn clip_switch_activates_inertialization_with_real_bones() {
        let mut canvas = NodeCanvas::default();
        canvas.anim_time = 0.7;
        canvas.set_active_clip(1, 1.0 / 60.0);
        assert!(canvas.inertializer.is_active());
        assert_eq!(canvas.inertializer.diffs.len(), canvas.rig.skeleton.len());
        assert_eq!(canvas.active_clip, 1);
        assert_eq!(canvas.anim_time, 0.0);
    }

    #[test]
    fn labels_track_the_active_clip() {
        let mut canvas = NodeCanvas::default();
        canvas.load_anim_demo();
        canvas.set_active_clip(1, 1.0 / 60.0);
        canvas.refresh_anim_labels();
        let (a, b) = canvas.blend_clip_names();
        assert_eq!(canvas.graph.nodes[0].kind, format!("Clip:{a}"));
        assert_eq!(canvas.graph.nodes[1].kind, format!("Clip:{b}"));
    }

    /// Runs the whole AnimGraph UI headless: it must lay out, draw the skeleton preview
    /// (one line per bone plus joints) and not panic.
    #[test]
    fn anim_graph_ui_draws_skeleton_preview_headless() {
        let mut canvas = NodeCanvas::default();
        canvas.load_anim_demo();
        canvas.anim_playing = true;
        let ctx = egui::Context::default();
        let mut shapes_with_preview = 0;
        for _ in 0..3 {
            let output = ctx.run(egui::RawInput::default(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    canvas.show(ui, None, None, None, None, 1.0 / 60.0);
                });
            });
            shapes_with_preview = output.shapes.len();
        }
        let bones = canvas.rig.skeleton.len();
        assert!(shapes_with_preview >= bones * 2, "only {shapes_with_preview} shapes drawn");
    }
}
