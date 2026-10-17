//! State tree transitions and gameplay primitives.

use redox_gameplay::state_tree::{StateTreeBuilder, StateTreeContext, TransitionCondition};
use redox_gameplay::{Ability, Attribute, GameplayEffect};

#[test]
fn conditions_evaluate_against_the_blackboard() {
    let mut ctx = StateTreeContext::new();
    ctx.set_flag("alert", true);
    ctx.set_float("noise", 0.7);
    assert!(TransitionCondition::FlagSet("alert").evaluate(&ctx));
    assert!(!TransitionCondition::FlagClear("alert").evaluate(&ctx));
    assert!(TransitionCondition::FloatAtLeast("noise", 0.5).evaluate(&ctx));
    assert!(TransitionCondition::FloatAtMost("noise", 0.9).evaluate(&ctx));
    assert!(!TransitionCondition::FlagSet("unknown").evaluate(&ctx));
    let both = TransitionCondition::And(
        Box::new(TransitionCondition::Always),
        Box::new(TransitionCondition::Not(Box::new(
            TransitionCondition::Never,
        ))),
    );
    assert!(both.evaluate(&ctx));
}

#[test]
fn builder_makes_the_first_state_the_root() {
    let mut b = StateTreeBuilder::new();
    let idle = b.add_state("idle");
    let run = b.add_state("run");
    b.add_transition(idle, run, TransitionCondition::FlagSet("go"));
    let tree = b.build();
    assert_eq!(tree.root(), idle);
    assert_eq!(tree.active_name(), "idle");
    assert!(tree.node(run).is_some());
}

#[test]
fn tree_transitions_when_the_guard_becomes_true() {
    let mut b = StateTreeBuilder::new();
    let idle = b.add_state("idle");
    let run = b.add_state("run");
    b.add_transition(idle, run, TransitionCondition::FlagSet("go"));
    let mut comp = redox_gameplay::StateTreeComponent::new(b.build());

    let r = comp.tree.tick(&mut comp.ctx, 0.1);
    assert!(!r.transitioned);
    assert!((comp.ctx.elapsed_in_state - 0.1).abs() < 1e-6);

    comp.ctx.set_flag("go", true);
    let r = comp.tree.tick(&mut comp.ctx, 0.1);
    assert!(r.transitioned);
    assert_eq!(r.entered, Some(run));
    assert_eq!(r.exited, Some(idle));
    assert_eq!(comp.tree.active_name(), "run");
}

#[test]
fn attributes_clamp_and_abilities_cool_down() {
    let mut hp = Attribute::new(100.0);
    hp.current = 250.0;
    hp.clamp();
    assert_eq!(hp.current, 100.0);
    hp.current = -5.0;
    hp.clamp();
    assert_eq!(hp.current, 0.0);

    let mut dash = Ability::new(1, 2.0, 10.0);
    assert!(dash.ready());
    dash.remaining_cooldown = 1.0;
    assert!(!dash.ready());
}

#[test]
fn effects_report_when_they_are_active() {
    assert!(GameplayEffect::instant_heal(10.0).active());
    let mut dot = GameplayEffect::dot(5.0, 2.0);
    assert!(dot.active());
    assert_eq!(dot.health_delta_per_sec, -5.0);
    dot.remaining = 0.0;
    assert!(!dot.active());
}
