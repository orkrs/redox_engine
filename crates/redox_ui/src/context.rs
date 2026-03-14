//! Core UI context — wraps egui + wgpu + winit integration.
//!
//! [`UiContext`] owns:
//! - The egui rendering pipeline (`egui_winit` + `egui_wgpu`)
//! - The [`DebugOverlay`] (stats panel + entity inspector)
//!
//! # Frame lifecycle
//! ```rust,ignore
//! // 1. Record timing data (before UI construction):
//! ui_ctx.debug.record_frame(dt);
//!
//! // 2. Begin egui frame:
//! ui_ctx.begin_frame(&window);
//!
//! // 3. Build any custom widgets, then draw the debug overlay:
//! ui_ctx.draw_debug(&world);   // or build widgets first, then call this
//!
//! // 4. Finalize and render:
//! ui_ctx.end_frame_and_render(device, queue, encoder, window, view, screen_desc);
//! ```

use crate::debug::DebugOverlay;
use redox_ecs::world::World;

/// ECS resource that owns the egui rendering integration.
pub struct UiContext {
    pub egui_ctx: egui::Context,
    egui_state: egui_winit::State,
    egui_renderer: egui_wgpu::Renderer,

    /// Whether the debug overlay master switch is on.
    pub show_debug: bool,

    /// The debug overlay (stats + inspector).  Drive it with
    /// [`record_frame`](Self::record_frame) + [`draw_debug`](Self::draw_debug).
    pub debug: DebugOverlay,
}

impl UiContext {
    /// Creates a new UI context for the given window and wgpu device.
    pub fn new(
        window: &winit::window::Window,
        device: &wgpu::Device,
        surface_format: wgpu::TextureFormat,
    ) -> Self {
        let egui_ctx = egui::Context::default();

        let egui_state = egui_winit::State::new(
            egui_ctx.clone(),
            egui::ViewportId::ROOT,
            window,
            Some(window.scale_factor() as f32),
            None,
        );

        let egui_renderer = egui_wgpu::Renderer::new(device, surface_format, None, 1);

        Self {
            egui_ctx,
            egui_state,
            egui_renderer,
            show_debug: true,
            debug: DebugOverlay::new(),
        }
    }

    // ── input ────────────────────────────────────────────────────────────────

    /// Forwards a winit event to egui. Returns `true` if egui consumed the event.
    pub fn handle_window_event(
        &mut self,
        window: &winit::window::Window,
        event: &winit::event::WindowEvent,
    ) -> bool {
        let response = self.egui_state.on_window_event(window, event);
        response.consumed
    }

    // ── frame lifecycle ───────────────────────────────────────────────────────

    /// Records one frame's delta time (seconds) into the stats panel.
    ///
    /// Call this **before** [`begin_frame`](Self::begin_frame).
    #[inline]
    pub fn record_frame(&mut self, dt: f32) {
        self.debug.record_frame(dt);
    }

    /// Begins a new egui frame. Call before building any UI.
    pub fn begin_frame(&mut self, window: &winit::window::Window) {
        let raw_input = self.egui_state.take_egui_input(window);
        self.egui_ctx.begin_frame(raw_input);
    }

    /// Returns the egui context for building custom UI widgets.
    pub fn ctx(&self) -> &egui::Context {
        &self.egui_ctx
    }

    /// Draws all debug overlay windows (stats + inspector).
    ///
    /// Call inside the egui frame (after [`begin_frame`](Self::begin_frame)).
    /// The overlay respects the [`show_debug`](Self::show_debug) flag —
    /// if `false`, nothing is drawn.
    pub fn draw_debug(&mut self, world: &World) {
        if !self.show_debug {
            return;
        }
        self.debug.show(&self.egui_ctx, world);
    }

    /// Ends the egui frame and renders into the given render pass.
    ///
    /// This must be called after `begin_frame` and after all UI widgets
    /// have been created for this frame.
    pub fn end_frame_and_render(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        window: &winit::window::Window,
        surface_view: &wgpu::TextureView,
        screen_descriptor: egui_wgpu::ScreenDescriptor,
    ) {
        let full_output = self.egui_ctx.end_frame();

        self.egui_state
            .handle_platform_output(window, full_output.platform_output);

        let tris = self
            .egui_ctx
            .tessellate(full_output.shapes, full_output.pixels_per_point);

        for (id, image_delta) in &full_output.textures_delta.set {
            self.egui_renderer
                .update_texture(device, queue, *id, image_delta);
        }

        self.egui_renderer
            .update_buffers(device, queue, encoder, &tris, &screen_descriptor);

        {
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("egui_render_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: surface_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load, // draw on top of the 3D scene
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });

            self.egui_renderer
                .render(&mut render_pass, &tris, &screen_descriptor);
        }

        for id in &full_output.textures_delta.free {
            self.egui_renderer.free_texture(id);
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn egui_context_standalone() {
        let ctx = egui::Context::default();
        assert!(ctx.style().spacing.item_spacing.x > 0.0);
    }
}
