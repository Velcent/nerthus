//! The SRT rotate handle uses screen pixels and wraps at the monitor's
//! top and bottom (`NativeMethods.TryWrapCursorVertically`). Read and
//! move the native pointer on the main thread, including while it is
//! outside the client area, then pass the result to that window's UI.

use bevy::camera::RenderTarget;
use bevy::ecs::system::NonSendMarker;
use bevy::prelude::*;
use bevy::window::{CursorMoved, Monitor, PrimaryWindow, RawHandleWrapper, WindowEvent, WindowRef};
use bevy::winit::WINIT_WINDOWS;
use bevy_egui::{EguiContext, EguiPreUpdateSet, egui};
use egui::{Id, Pos2, Rect};

const SCREEN_MARGIN: f32 = 8.0;
const ROTATE_STEP: f32 = 0.5;
const ROTATE_SNAP: f32 = 15.0;

pub struct GizmoPointerPlugin;

impl Plugin for GizmoPointerPlugin {
    fn build(&self, app: &mut App) {
        let pointer_system =
            follow_rotation.after(EguiPreUpdateSet::InitContexts).after(bevy::input::InputSystems).before(EguiPreUpdateSet::ProcessInput);
        app.add_systems(PreUpdate, pointer_system);
    }
}

/// Relative to the window's content origin, in physical screen pixels.
#[derive(Clone, Copy, Debug)]
pub(super) struct PointerFrame {
    pub position: Pos2,
    pub wrapped: bool,
}

/// Both SRT and Constructor gizmos turn by native vertical displacement.
/// Keep the unsnapped angle when the pointer moves to the opposite edge.
#[derive(Clone, Debug)]
pub(super) struct RotationDrag {
    start_pointer: Pos2,
    start_degrees: f32,
    raw_degrees: f32,
}

impl RotationDrag {
    pub fn new(pointer: Pos2, degrees: f32) -> Self {
        Self { start_pointer: pointer, start_degrees: degrees, raw_degrees: degrees }
    }

    pub fn rotate_to(&mut self, pointer: Pos2, wrapped: bool, modifiers: egui::Modifiers) -> Option<f32> {
        if wrapped {
            self.start_pointer = pointer;
            self.start_degrees = self.raw_degrees;
            return None;
        }
        let step = if modifiers.shift { ROTATE_STEP * 0.25 } else { ROTATE_STEP };
        let movement = pointer.y - self.start_pointer.y;
        self.raw_degrees = (self.start_degrees - movement * step).rem_euclid(360.0);
        let shown = if modifiers.ctrl || modifiers.command { (self.raw_degrees / ROTATE_SNAP).round() * ROTATE_SNAP } else { self.raw_degrees };
        Some(shown.rem_euclid(360.0))
    }
}

#[derive(Clone, Default)]
struct PointerState {
    rotating: bool,
    last_screen_position: Option<Pos2>,
    frame: Option<PointerFrame>,
}

fn state_id() -> Id {
    Id::new("gizmo-screen-pointer")
}

pub(super) fn request_rotation(ctx: &egui::Context) {
    ctx.data_mut(|data| {
        let state = data.get_temp_mut_or_default::<PointerState>(state_id());
        // Several gizmos can be drawn in this context. An idle gizmo
        // cannot cancel another one's request; PreUpdate clears it.
        state.rotating = true;
    });
}

pub(super) fn pointer_frame(ctx: &egui::Context) -> Option<PointerFrame> {
    let state = ctx.data(|data| data.get_temp::<PointerState>(state_id()));
    state.and_then(|state| state.frame)
}

fn follow_rotation(
    _main_thread: NonSendMarker,
    mut contexts: Query<(&RenderTarget, &mut EguiContext)>,
    windows: Query<(&Window, &RawHandleWrapper)>,
    primary: Query<Entity, With<PrimaryWindow>>,
    monitors: Query<&Monitor>,
    mouse_buttons: Res<ButtonInput<MouseButton>>,
    mut events: MessageWriter<WindowEvent>,
) {
    let primary = primary.single().ok();
    for (target, mut context) in &mut contexts {
        let ctx = context.get_mut();
        let mut state = ctx.data_mut(|data| data.get_temp::<PointerState>(state_id()).unwrap_or_default());
        let rotating = state.rotating && mouse_buttons.pressed(MouseButton::Left);
        // A hidden, closed, or replaced gizmo stops requesting the
        // pointer. A drawn rotate drag renews this for the next frame.
        state.rotating = false;
        state.frame = None;

        let window_entity = match target {
            RenderTarget::Window(WindowRef::Primary) => primary,
            RenderTarget::Window(WindowRef::Entity(entity)) => Some(*entity),
            _ => None,
        };
        if let Some(entity) = window_entity
            && let Ok((window, handles)) = windows.get(entity)
        {
            let rotating = rotating && window.focused;
            state.frame = read_pointer(entity, window, handles, &monitors, rotating, &mut state.last_screen_position);
            if rotating && let Some(frame) = state.frame {
                let position = logical_position(frame.position, window.scale_factor());
                let moved = CursorMoved { window: entity, position, delta: None };
                events.write(WindowEvent::CursorMoved(moved));
            }
        }
        ctx.data_mut(|data| data.insert_temp(state_id(), state));
    }
}

fn read_pointer(
    entity: Entity,
    window: &Window,
    handles: &RawHandleWrapper,
    monitors: &Query<&Monitor>,
    rotating: bool,
    last_screen_position: &mut Option<Pos2>,
) -> Option<PointerFrame> {
    let logical = crate::drop_position::system_pointer::position(handles, window.scale_factor())?;
    let position = Pos2::new(logical.x, logical.y) * window.scale_factor();
    WINIT_WINDOWS.with_borrow(|native_windows| {
        let native_window = native_windows.get_window(entity)?;
        let origin = native_window.inner_position().ok()?;
        let origin = egui::vec2(origin.x as f32, origin.y as f32);
        let screen_position = position + origin;
        let moved = *last_screen_position != Some(screen_position);
        let screen = monitor_bounds(screen_position, monitors, window.scale_factor());
        let target = screen.and_then(|(bounds, pixel_scale)| vertical_wrap_target(screen_position, bounds, pixel_scale));
        let mut frame = PointerFrame { position, wrapped: false };
        if rotating
            && moved
            && let Some(target) = target
        {
            let relative = target - origin;
            // The Bevy Window setter rejects positions outside its
            // content area; the destination can be anywhere on the
            // monitor, including beyond a restored window's edges.
            let mut native_target = native_window.inner_position().ok()?;
            native_target.x = relative.x.round() as i32;
            native_target.y = relative.y.round() as i32;
            if native_window.set_cursor_position(native_target).is_ok() {
                // The OS can queue the synthetic move event. Rebase to
                // the accepted destination even if a fresh native read
                // would still return the source for this instant.
                frame.position = Pos2::new(native_target.x as f32, native_target.y as f32);
                frame.wrapped = true;
            }
        }
        // Ignore the warp's own synthetic movement event at the next
        // frame, which would immediately wrap back at the other edge.
        *last_screen_position = Some(frame.position + origin);
        Some(frame)
    })
}

fn logical_position(physical: Pos2, scale_factor: f32) -> Vec2 {
    Vec2::new(physical.x, physical.y) / scale_factor
}

fn monitor_bounds(position: Pos2, monitors: &Query<&Monitor>, window_scale: f32) -> Option<(Rect, f32)> {
    let mut nearest = None;
    let mut least_distance = f32::INFINITY;
    for monitor in monitors {
        let origin = monitor.physical_position;
        let size = monitor.physical_size();
        // macOS has a shared desktop in logical pixels; winit scales
        // each monitor's desktop coordinates by that monitor's factor.
        // Convert them to the window's physical coordinate system.
        let pixel_scale = if cfg!(target_os = "macos") { window_scale / monitor.scale_factor as f32 } else { 1.0 };
        let min = Pos2::new(origin.x as f32, origin.y as f32) * pixel_scale;
        let size = egui::vec2(size.x as f32, size.y as f32) * pixel_scale;
        let bounds = Rect::from_min_size(min, size);
        let contains = position.x >= bounds.left() && position.x < bounds.right() && position.y >= bounds.top() && position.y < bounds.bottom();
        if contains {
            return Some((bounds, pixel_scale));
        }
        let distance = bounds.distance_sq_to_pos(position);
        if distance < least_distance {
            nearest = Some((bounds, pixel_scale));
            least_distance = distance;
        }
    }
    nearest
}

/// Eight physical pixels inside the current screen, as in Gaea. Only
/// the rotation handle asks for this; other drags retain their axes.
fn vertical_wrap_target(position: Pos2, bounds: Rect, pixel_scale: f32) -> Option<Pos2> {
    let margin = SCREEN_MARGIN * pixel_scale;
    if bounds.width() <= margin * 2.0 + pixel_scale || bounds.height() <= margin * 2.0 + pixel_scale {
        return None;
    }
    let top = bounds.top() + margin;
    let bottom = bounds.bottom() - pixel_scale - margin;
    let y = if position.y <= top {
        bottom
    } else if position.y >= bottom {
        top
    } else {
        return None;
    };
    let x = position.x.clamp(bounds.left() + margin, bounds.right() - pixel_scale - margin);
    Some(Pos2::new(x, y))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn closed_or_replaced_viewport_contexts_stop_requesting_cursor_wraps() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.add_message::<WindowEvent>();
        app.init_resource::<ButtonInput<MouseButton>>();
        app.add_plugins(GizmoPointerPlugin);

        let mut context = EguiContext::default();
        let ctx = context.get_mut().clone();
        request_rotation(&ctx);
        let target = RenderTarget::Window(WindowRef::Primary);
        app.world_mut().spawn((context, target));

        app.update();
        let state = ctx.data(|data| data.get_temp::<PointerState>(state_id()).unwrap());
        assert!(!state.rotating);
        assert!(state.frame.is_none());
    }

    #[test]
    fn wraps_at_the_monitors_vertical_edges_in_both_directions() {
        let bounds = Rect::from_min_size(Pos2::ZERO, egui::vec2(1920.0, 1080.0));
        assert_eq!(vertical_wrap_target(egui::pos2(400.0, 8.0), bounds, 1.0), Some(egui::pos2(400.0, 1071.0)));
        assert_eq!(vertical_wrap_target(egui::pos2(400.0, 1071.0), bounds, 1.0), Some(egui::pos2(400.0, 8.0)));
        assert_eq!(vertical_wrap_target(egui::pos2(400.0, 9.0), bounds, 1.0), None);
        assert_eq!(vertical_wrap_target(egui::pos2(400.0, 1070.0), bounds, 1.0), None);
        // Crossing a horizontal edge alone does not wrap.
        assert_eq!(vertical_wrap_target(egui::pos2(0.0, 500.0), bounds, 1.0), None);
    }

    #[test]
    fn chooses_the_monitor_containing_the_pointer_even_at_a_shared_edge() {
        let mut world = World::new();
        for x in [-1920, 0] {
            let monitor = Monitor {
                name: None,
                physical_height: 1080,
                physical_width: 1920,
                physical_position: IVec2::new(x, 0),
                refresh_rate_millihertz: None,
                scale_factor: 1.0,
                video_modes: Vec::new(),
            };
            world.spawn(monitor);
        }
        let mut queries = bevy::ecs::system::SystemState::<Query<&Monitor>>::new(&mut world);
        let monitors = queries.get(&world).unwrap();
        let (left, _) = monitor_bounds(egui::pos2(-1.0, 10.0), &monitors, 1.0).unwrap();
        let (right, _) = monitor_bounds(egui::pos2(0.0, 10.0), &monitors, 1.0).unwrap();
        assert_eq!(left.left(), -1920.0);
        assert_eq!(right.left(), 0.0);
    }

    #[test]
    fn wraps_on_the_pointer_monitor_with_negative_coordinates_and_corner_insets() {
        let bounds = Rect::from_min_size(egui::pos2(-2560.0, -1440.0), egui::vec2(2560.0, 1440.0));
        assert_eq!(vertical_wrap_target(egui::pos2(-2560.0, -1440.0), bounds, 1.0), Some(egui::pos2(-2552.0, -9.0)));
        assert_eq!(vertical_wrap_target(egui::pos2(-1.0, -1.0), bounds, 1.0), Some(egui::pos2(-9.0, -1432.0)));
    }

    #[test]
    fn screen_insets_follow_the_pointer_monitors_physical_pixel_scale() {
        // A 1x monitor seen from a 2x window has doubled coordinates.
        let bounds = Rect::from_min_size(egui::pos2(-2560.0, 0.0), egui::vec2(2560.0, 1440.0));
        let target = vertical_wrap_target(egui::pos2(-1000.0, 16.0), bounds, 2.0);
        assert_eq!(target, Some(egui::pos2(-1000.0, 1422.0)));
        assert_eq!(vertical_wrap_target(egui::pos2(-1000.0, 18.0), bounds, 2.0), None);
    }

    #[test]
    fn native_pixels_are_independent_of_interface_zoom() {
        let physical = egui::pos2(240.0, 1200.0);
        let logical = logical_position(physical, 2.0);
        assert_eq!(logical, Vec2::new(120.0, 600.0));
        // egui's interface zoom is applied after native pixel -> window
        // logical pixel conversion by bevy_egui, never by cursor warping.
        let egui_position = logical / 1.5;
        assert_eq!(egui_position, Vec2::new(80.0, 400.0));
        assert_eq!(egui_position * 3.0, Vec2::new(physical.x, physical.y));
    }

    #[test]
    fn rotation_continues_across_multiple_upward_and_downward_screen_wraps() {
        let mut drag = RotationDrag::new(Pos2::new(400.0, 400.0), 350.0);
        let plain = egui::Modifiers::NONE;
        assert_eq!(drag.rotate_to(Pos2::new(400.0, 380.0), false, plain), Some(0.0));

        // Screen jumps preserve the last angle. Motion resumes from
        // each new pointer origin in either direction.
        assert_eq!(drag.rotate_to(Pos2::new(400.0, 1071.0), true, plain), None);
        assert_eq!(drag.raw_degrees, 0.0);
        assert_eq!(drag.rotate_to(Pos2::new(400.0, 1001.0), false, plain), Some(35.0));
        assert_eq!(drag.rotate_to(Pos2::new(400.0, 1071.0), true, plain), None);
        assert_eq!(drag.rotate_to(Pos2::new(400.0, 1021.0), false, plain), Some(60.0));

        assert_eq!(drag.rotate_to(Pos2::new(400.0, 8.0), true, plain), None);
        assert_eq!(drag.rotate_to(Pos2::new(400.0, 108.0), false, plain), Some(10.0));
        assert_eq!(drag.rotate_to(Pos2::new(400.0, 8.0), true, plain), None);
        assert_eq!(drag.rotate_to(Pos2::new(400.0, 48.0), false, plain), Some(350.0));
    }

    #[test]
    fn rotation_wraps_preserve_unsnapped_values_and_shift_precision() {
        let mut drag = RotationDrag::new(Pos2::new(400.0, 400.0), 10.0);
        let snapped = egui::Modifiers::CTRL;
        assert_eq!(drag.rotate_to(Pos2::new(400.0, 394.0), false, snapped), Some(15.0));
        assert_eq!(drag.raw_degrees, 13.0);
        assert_eq!(drag.rotate_to(Pos2::new(400.0, 1071.0), true, snapped), None);
        assert_eq!(drag.raw_degrees, 13.0);
        assert_eq!(drag.rotate_to(Pos2::new(400.0, 1071.0), false, egui::Modifiers::NONE), Some(13.0));

        let fine = egui::Modifiers::SHIFT;
        assert_eq!(drag.rotate_to(Pos2::new(800.0, 1063.0), false, fine), Some(14.0));
        // Horizontal displacement cannot change a vertical rotate drag.
        assert_eq!(drag.rotate_to(Pos2::new(10.0, 1063.0), false, fine), Some(14.0));
        assert_eq!(drag.rotate_to(Pos2::new(10.0, 8.0), true, fine), None);
        assert_eq!(drag.rotate_to(Pos2::new(10.0, 16.0), false, fine), Some(13.0));
    }
}
