//! Deferred command buffer behaviour.

use redox_ecs::{CommandQueue, Commands, Query, World};

#[derive(Debug, Clone, PartialEq)]
struct Hp(i32);
#[derive(Debug, Clone, PartialEq)]
struct Tag;
struct Score(u32);

#[test]
fn spawn_is_deferred_until_apply_and_id_is_usable_immediately() {
    let mut world = World::new();
    let mut queue = CommandQueue::new();

    let e = {
        let mut cmds = Commands::new(&world, &mut queue);
        let e = cmds.spawn((Hp(10),));
        cmds.insert(e, Tag); // refers to the reserved id before it exists
        e
    };
    assert_eq!(queue.len(), 2);
    assert!(
        world.get_component::<Hp>(e).is_none(),
        "nothing happens before apply"
    );

    queue.apply(&mut world);
    assert!(queue.is_empty());
    assert_eq!(world.get_component::<Hp>(e), Some(&Hp(10)));
    assert_eq!(world.get_component::<Tag>(e), Some(&Tag));
}

#[test]
fn despawn_and_remove_run_in_recorded_order() {
    let mut world = World::new();
    let a = world.spawn_bundle((Hp(1), Tag));
    let b = world.spawn_bundle((Hp(2),));

    let mut queue = CommandQueue::new();
    {
        let mut cmds = Commands::new(&world, &mut queue);
        cmds.remove::<Tag>(a);
        cmds.despawn(b);
        cmds.insert_resource(Score(7));
    }
    queue.apply(&mut world);

    assert!(world.get_component::<Tag>(a).is_none());
    assert_eq!(world.get_component::<Hp>(a), Some(&Hp(1)));
    assert!(world.get_component::<Hp>(b).is_none());
    assert_eq!(world.get_resource::<Score>().map(|s| s.0), Some(7));
}

#[test]
fn commands_recorded_while_iterating_a_query() {
    let mut world = World::new();
    for hp in [5, 0, 3, 0] {
        world.spawn_bundle((Hp(hp),));
    }

    let mut queue = CommandQueue::new();
    {
        let mut cmds = Commands::new(&world, &mut queue);
        for (e, hp) in Query::<(redox_ecs::Entity, &Hp)>::new().iter(&world) {
            if hp.0 == 0 {
                cmds.despawn(e);
            }
        }
    }
    queue.apply(&mut world);
    assert_eq!(Query::<&Hp>::new().iter(&world).count(), 2);
}

#[test]
fn custom_command_and_stale_despawn_are_harmless() {
    let mut world = World::new();
    let e = world.spawn_bundle((Hp(1),));
    let mut queue = CommandQueue::new();
    {
        let mut cmds = Commands::new(&world, &mut queue);
        cmds.despawn(e);
        cmds.despawn(e); // second one targets a stale handle
        cmds.add(|w| {
            w.insert_resource(Score(1));
        });
    }
    queue.apply(&mut world);
    assert!(world.get_component::<Hp>(e).is_none());
    assert!(world.get_resource::<Score>().is_some());
}
