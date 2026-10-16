# ADR 0003: Target dependency versions (draft)

Status: accepted

wgpu, winit and egui (with egui-wgpu / egui-winit) must move together, one wgpu major
version at a time. Record the chosen versions, the date, and the crates.io check here.
Current (2026-10-17): wgpu 30, naga 30, winit 0.30, egui 0.36 (egui_dock 0.21, egui_plot 0.37 target egui 0.36, so egui 0.37 is not usable yet), rapier3d 0.36, kira 0.12, glam 0.33 (kira pins 0.33; 0.34 removed the mint feature). Toolchain 1.96.0, MSRV 1.95 (egui 0.36).
