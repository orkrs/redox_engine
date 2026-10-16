# ADR 0002: Own archetype ECS

Status: accepted

## Context
The engine needs queries with change detection, parallel iteration and tight control of
memory layout for rendering. Existing crates (bevy_ecs, hecs, legion) were not adopted.

## Decision
Keep the in-tree `redox_ecs` (archetype tables, column storage, tuple queries). Compensate
for the risk of hand-written `unsafe` with documented invariants, Miri in CI, and property tests.

## Consequences
Full control, but the project owns soundness. Commands, `par_iter_mut` and scheduling are
tracked in roadmap 1.3.
