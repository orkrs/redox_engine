# ADR 0001: Virtual shadow maps replace cascaded shadow maps

Status: accepted (implemented in `9bcf961`)

## Context
Sun shadows had two paths: CSM (4 cascades) and a VSM clipmap. CSM depth was disabled
(`render_csm_depth = false`) but its textures, bindings, uniform and shader branches stayed,
costing memory and making the shaders hard to read.

## Decision
Directional shadows use only the VSM clipmap. CSM code, bindings 9/10/12 of the PBR scene
group, the `ShadowUniform`, and the `shadow_probe` example were deleted. With
`soft_shadows != 2` the sun is unshadowed.

## Consequences
Smaller shaders and bind groups; golden frames unchanged. Hard-shadow fallback for devices
without enough texture layers is not available (VSM needs a layered page pool).
