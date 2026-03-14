#[cfg(test)]
mod ecs_core_tests {
    use redox_ecs::*;

    #[derive(Debug, PartialEq, Clone)]
    struct Position(f32, f32, f32);

    #[derive(Debug, PartialEq, Clone)]
    struct Velocity(f32, f32, f32);

    #[derive(Debug, PartialEq, Clone)]
    struct Health(f32);

    #[derive(Debug, PartialEq, Clone)]
    struct Name(String);

    /// Marker component used to test With/Without filters.
    #[derive(Debug, Clone)]
    struct Player;

    #[test]
    fn test_entity_allocation() {
        let mut world = World::new();
        let e1 = world.spawn();
        let e2 = world.spawn();
        assert_ne!(e1.id(), e2.id());
        assert_eq!(e1.generation(), 0);
        assert_eq!(e2.generation(), 0);
    }

    // -----------------------------------------------------------------------
    // Single-component queries (backward-compat)
    // -----------------------------------------------------------------------

    #[test]
    fn test_add_component_and_query() {
        let mut world = World::new();
        let entity = world.spawn();
        world.add_component(entity, Position(1.0, 2.0, 3.0));
        world.add_component(entity, Health(100.0));

        let query = Query::<&Position>::new();
        let positions: Vec<&Position> = query.iter(&world).collect();
        assert_eq!(positions.len(), 1);
        assert_eq!(*positions[0], Position(1.0, 2.0, 3.0));

        let health_query = Query::<&Health>::new();
        let healths: Vec<&Health> = health_query.iter(&world).collect();
        assert_eq!(healths.len(), 1);
        assert_eq!(*healths[0], Health(100.0));
    }

    #[test]
    fn test_remove_component() {
        let mut world = World::new();
        let entity = world.spawn();
        world.add_component(entity, Position(1.0, 2.0, 3.0));
        world.add_component(entity, Health(100.0));

        let removed = world.remove_component::<Health>(entity);
        assert!(removed);

        let health_query = Query::<&Health>::new();
        let healths: Vec<&Health> = health_query.iter(&world).collect();
        assert_eq!(healths.len(), 0);

        let pos_query = Query::<&Position>::new();
        let positions: Vec<&Position> = pos_query.iter(&world).collect();
        assert_eq!(positions.len(), 1);
    }

    #[test]
    fn test_despawn() {
        let mut world = World::new();
        let entity = world.spawn();
        world.add_component(entity, Position(1.0, 2.0, 3.0));

        world.despawn(entity);

        let pos_query = Query::<&Position>::new();
        let positions: Vec<&Position> = pos_query.iter(&world).collect();
        assert_eq!(positions.len(), 0);
    }

    // -----------------------------------------------------------------------
    // Tuple queries – two components
    // -----------------------------------------------------------------------

    #[test]
    fn test_tuple_query_two_components() {
        let mut world = World::new();

        let e1 = world.spawn();
        world.add_component(e1, Position(1.0, 0.0, 0.0));
        world.add_component(e1, Velocity(10.0, 0.0, 0.0));

        let e2 = world.spawn();
        world.add_component(e2, Position(2.0, 0.0, 0.0));
        // e2 has no Velocity → should NOT appear in the tuple query

        let q = Query::<(&Position, &Velocity)>::new();
        let results: Vec<_> = q.iter(&world).collect();

        assert_eq!(results.len(), 1, "only e1 has both Position and Velocity");
        let (pos, vel) = results[0];
        assert_eq!(*pos, Position(1.0, 0.0, 0.0));
        assert_eq!(*vel, Velocity(10.0, 0.0, 0.0));
    }

    #[test]
    fn test_tuple_query_three_components() {
        let mut world = World::new();

        let e1 = world.spawn();
        world.add_component(e1, Position(1.0, 2.0, 3.0));
        world.add_component(e1, Velocity(4.0, 5.0, 6.0));
        world.add_component(e1, Health(100.0));

        let e2 = world.spawn();
        world.add_component(e2, Position(0.0, 0.0, 0.0));
        world.add_component(e2, Velocity(1.0, 0.0, 0.0));
        // e2 has no Health

        let q = Query::<(&Position, &Velocity, &Health)>::new();
        let results: Vec<_> = q.iter(&world).collect();

        assert_eq!(results.len(), 1, "only e1 has all three components");
        let (pos, vel, hp) = results[0];
        assert_eq!(*pos, Position(1.0, 2.0, 3.0));
        assert_eq!(*vel, Velocity(4.0, 5.0, 6.0));
        assert_eq!(*hp, Health(100.0));
    }

    // -----------------------------------------------------------------------
    // world.query() and world.query_filtered() convenience methods
    // -----------------------------------------------------------------------

    #[test]
    fn test_world_query_convenience() {
        let mut world = World::new();
        let e = world.spawn();
        world.add_component(e, Position(7.0, 8.0, 9.0));
        world.add_component(e, Velocity(1.0, 2.0, 3.0));

        let results: Vec<_> = world
            .query::<(&Position, &Velocity)>()
            .iter(&world)
            .collect();
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn test_world_query_filtered_with() {
        let mut world = World::new();

        // Player entity: has Position + Player
        let player = world.spawn();
        world.add_component(player, Position(1.0, 0.0, 0.0));
        world.add_component(player, Player);

        // Non-player entity: has Position but no Player
        let npc = world.spawn();
        world.add_component(npc, Position(2.0, 0.0, 0.0));

        // Only the player entity should be returned
        let results: Vec<_> = world
            .query_filtered::<&Position, With<Player>>()
            .iter(&world)
            .collect();

        assert_eq!(results.len(), 1);
        assert_eq!(*results[0], Position(1.0, 0.0, 0.0));
    }

    #[test]
    fn test_world_query_filtered_without() {
        let mut world = World::new();

        // Living entity: has Position, no Health component removed
        let alive = world.spawn();
        world.add_component(alive, Position(1.0, 0.0, 0.0));
        world.add_component(alive, Health(50.0));

        // Dead entity: Position only (no Health)
        let dead = world.spawn();
        world.add_component(dead, Position(2.0, 0.0, 0.0));

        // Without<Health> → only the dead entity (no Health component)
        let results: Vec<_> = world
            .query_filtered::<&Position, Without<Health>>()
            .iter(&world)
            .collect();

        assert_eq!(results.len(), 1);
        assert_eq!(*results[0], Position(2.0, 0.0, 0.0));
    }

    #[test]
    fn test_filtered_tuple_query() {
        let mut world = World::new();

        // Entity with all three components
        let full = world.spawn();
        world.add_component(full, Position(1.0, 0.0, 0.0));
        world.add_component(full, Velocity(1.0, 0.0, 0.0));
        world.add_component(full, Player);

        // Entity with Position + Velocity but no Player marker
        let partial = world.spawn();
        world.add_component(partial, Position(2.0, 0.0, 0.0));
        world.add_component(partial, Velocity(2.0, 0.0, 0.0));

        let results: Vec<_> = world
            .query_filtered::<(&Position, &Velocity), With<Player>>()
            .iter(&world)
            .collect();

        assert_eq!(results.len(), 1, "only the full entity passes the filter");
        assert_eq!(*results[0].0, Position(1.0, 0.0, 0.0));
    }

    // -----------------------------------------------------------------------
    // Mutable tuple query
    // -----------------------------------------------------------------------

    #[test]
    fn test_mutable_single_component_query() {
        let mut world = World::new();
        let e = world.spawn();
        world.add_component(e, Health(100.0));

        // Mutably iterate and modify
        {
            let q = Query::<&mut Health>::new();
            for hp in q.iter(&world) {
                hp.0 -= 10.0;
            }
        }

        assert_eq!(world.get_component::<Health>(e).unwrap().0, 90.0);
    }

    #[test]
    fn test_mutable_tuple_query() {
        let mut world = World::new();
        let e = world.spawn();
        world.add_component(e, Position(0.0, 0.0, 0.0));
        world.add_component(e, Velocity(1.0, 2.0, 3.0));

        // Apply velocity to position
        {
            let q = Query::<(&mut Position, &Velocity)>::new();
            for (pos, vel) in q.iter(&world) {
                pos.0 += vel.0;
                pos.1 += vel.1;
                pos.2 += vel.2;
            }
        }

        let pos = world.get_component::<Position>(e).unwrap();
        assert_eq!(*pos, Position(1.0, 2.0, 3.0));
    }

    // -----------------------------------------------------------------------
    // Multiple entities in same archetype
    // -----------------------------------------------------------------------

    #[test]
    fn test_tuple_query_multiple_entities_same_archetype() {
        let mut world = World::new();

        for i in 0..5u32 {
            let e = world.spawn();
            world.add_component(e, Position(i as f32, 0.0, 0.0));
            world.add_component(e, Velocity(-(i as f32), 0.0, 0.0));
        }

        let q = Query::<(&Position, &Velocity)>::new();
        let results: Vec<_> = q.iter(&world).collect();
        assert_eq!(results.len(), 5);

        // Verify each pair: pos.x + vel.x == 0
        for (pos, vel) in &results {
            assert_eq!(pos.0 + vel.0, 0.0);
        }
    }

    // -----------------------------------------------------------------------
    // Tuple filter combination
    // -----------------------------------------------------------------------

    #[test]
    fn test_tuple_filter_with_and_without() {
        let mut world = World::new();

        // Entity: Position + Player (no Health)
        let player = world.spawn();
        world.add_component(player, Position(1.0, 0.0, 0.0));
        world.add_component(player, Player);

        // Entity: Position + Player + Health
        let healthy_player = world.spawn();
        world.add_component(healthy_player, Position(2.0, 0.0, 0.0));
        world.add_component(healthy_player, Player);
        world.add_component(healthy_player, Health(100.0));

        // Entity: Position only
        let other = world.spawn();
        world.add_component(other, Position(3.0, 0.0, 0.0));

        // Filter: With<Player> AND Without<Health>  → only `player`
        let results: Vec<_> = world
            .query_filtered::<&Position, (With<Player>, Without<Health>)>()
            .iter(&world)
            .collect();

        assert_eq!(results.len(), 1);
        assert_eq!(*results[0], Position(1.0, 0.0, 0.0));
    }

    // -----------------------------------------------------------------------
    // Hierarchy tests (unchanged)
    // -----------------------------------------------------------------------

    #[test]
    fn test_hierarchy() {
        let mut world = World::new();
        let parent = world.spawn();
        let child = world.spawn();

        world.add_component(parent, Name("Parent".to_string()));
        world.add_component(child, Name("Child".to_string()));
        world.add_component(child, Parent(parent));

        let query = Query::<&Parent>::new();
        let parents: Vec<&Parent> = query.iter(&world).collect();
        assert_eq!(parents.len(), 1);
        assert_eq!(parents[0].0, parent);
    }

    // -----------------------------------------------------------------------
    // Events (unchanged)
    // -----------------------------------------------------------------------

    #[test]
    fn test_events() {
        struct TestEvent(i32);
        let mut events = Events::<TestEvent>::new();
        events.send(TestEvent(42));
        events.update();

        let mut reader = EventReader::new(&events);
        let collected: Vec<i32> = reader.iter().map(|e| e.0).collect();
        assert_eq!(collected, vec![42]);
    }
}

// ---------------------------------------------------------------------------
// Parallel query tests (separate module to keep Rayon import scoped)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod parallel_query_tests {
    use redox_ecs::*;
    use redox_ecs::ParallelQuery;
    use rayon::iter::ParallelIterator;

    #[derive(Debug, Clone, PartialEq)]
    struct Mass(f32);

    #[derive(Debug, Clone, PartialEq)]
    struct Force(f32);

    #[test]
    fn test_par_iter_single_component() {
        let mut world = World::new();
        for i in 0..10u32 {
            let e = world.spawn();
            world.add_component(e, Mass(i as f32));
        }

        let q = Query::<&Mass>::new();
        let sum: f32 = q.par_iter(&world).map(|m| m.0).sum();
        // 0 + 1 + … + 9 = 45
        assert_eq!(sum, 45.0);
    }

    #[test]
    fn test_par_iter_tuple() {
        let mut world = World::new();
        for i in 0..4u32 {
            let e = world.spawn();
            world.add_component(e, Mass(i as f32));
            world.add_component(e, Force(2.0));
        }

        let q = Query::<(&Mass, &Force)>::new();
        let sum: f32 = q.par_iter(&world).map(|(m, f)| m.0 * f.0).sum();
        // (0*2 + 1*2 + 2*2 + 3*2) = 12
        assert_eq!(sum, 12.0);
    }
}
