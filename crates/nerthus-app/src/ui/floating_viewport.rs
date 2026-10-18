//! Viewport Options › Floating (`Shell.FloatViewport`): the viewport area
//! (its toolbar, the 3D view and the docked 2D view) in a window of its
//! own, "Nerthus - <file>", at the leftmost screen's top left and
//! maximized when there is more than one screen or it cannot fit, while
//! the graph takes the main window's whole left column. Embedded puts it back
//! (`Shell.EmbedViewport`). The window does not close on its own
//! (`ViewportWindow_Closing`), and the choice is kept in the options
//! (`ViewportConfig`), so Nerthus starts the way it was left.

use bevy::camera::visibility::RenderLayers;
use bevy::camera::{CameraOutputMode, RenderTarget};
use bevy::ecs::schedule::ScheduleLabel;
use bevy::prelude::*;
use bevy::render::render_resource::BlendState;
use bevy::window::{Monitor, WindowRef};
use bevy_egui::{egui, EguiContext, EguiSchedule, PrimaryEguiContext};
use egui::{LayerId, Ui, UiBuilder};
use nerthus_render::{NavigationSettings, StrategyCamera, ViewportCamera, ViewportInput, ViewportSettings};

use crate::editor::Editor;
use crate::theme;
use crate::ui::{fit_camera, surround, tool_window, viewport_area, UiState};

/// The floating window's egui pass.
#[derive(ScheduleLabel, Clone, Debug, PartialEq, Eq, Hash)]
pub struct FloatingViewportPass;

/// The floating window, and the camera its interface is drawn with.
#[derive(Component)]
pub struct FloatingViewportWindow;

#[derive(Component)]
pub struct FloatingViewportUi;

/// The window's size before it is maximized.
const WINDOW_SIZE: (u32, u32) = (1280, 800);
/// Room for the native frame and taskbar, in logical pixels. Maximizing
/// lets the window manager use its actual work area on smaller screens.
const WINDOW_FRAME_ALLOWANCE: Vec2 = Vec2::new(32.0, 80.0);
/// `ViewportArea`'s margin inside the window (`SquareBorders`).
const AREA_MARGIN: f32 = 2.0;

/// The window's title (`"Nerthus - " + file name`).
fn title(editor: &Editor) -> String {
    format!("Nerthus - {}", editor.document.file_name())
}

/// Keeps the normal size on large screens and a usable restore size on
/// small or scaled screens, even after the user unmaximizes the window.
fn window_on_monitor(window_title: String, monitor: Option<&Monitor>, monitor_count: usize) -> Window {
    let mut window = Window {
        title: window_title,
        resolution: WINDOW_SIZE.into(),
        ..default()
    };
    let Some(monitor) = monitor else {
        window.set_maximized(true);
        return window;
    };

    window.position = WindowPosition::At(monitor.physical_position);
    let logical_size = monitor.physical_size().as_vec2() / monitor.scale_factor as f32;
    let available_size = (logical_size - WINDOW_FRAME_ALLOWANCE).max(Vec2::ONE);
    let normal_size = Vec2::new(WINDOW_SIZE.0 as f32, WINDOW_SIZE.1 as f32);
    let restore_size = normal_size.min(available_size);
    window.resolution.set(restore_size.x, restore_size.y);

    let too_small = normal_size.x > available_size.x || normal_size.y > available_size.y;
    if monitor_count > 1 || too_small {
        window.set_maximized(true);
    }
    window
}

/// Opens the window when the options say Floating and closes it when they
/// say Embedded, moving the 3D view between the windows; keeps its title.
#[allow(clippy::too_many_arguments)]
pub fn manage(
    mut commands: Commands,
    editor: Res<Editor>,
    mut ui_state: ResMut<UiState>,
    mut windows: Query<(Entity, &mut Window), With<FloatingViewportWindow>>,
    interfaces: Query<Entity, With<FloatingViewportUi>>,
    cameras: Query<(Entity, &RenderTarget), Without<FloatingViewportUi>>,
    mut main_interface: Single<&mut Camera, With<PrimaryEguiContext>>,
    viewport_camera: Single<Entity, With<ViewportCamera>>,
    monitors: Query<&Monitor>,
) {
    let floating = editor.user.options.viewport_floating();
    let open = windows.iter().next().is_some();
    if floating == open {
        if let Some((_, mut window)) = windows.iter_mut().next() {
            let wanted = title(&editor);
            if window.title != wanted {
                window.title = wanted;
            }
        }
        return;
    }
    // The docked 2D view's pictures belong to the context it was drawn in.
    ui_state.viewport_2d.forget_pictures();
    // Embedded, the 3D view clears the main window before its interface
    // is drawn over it; floating, nothing else draws there.
    let main_clear = if floating { ClearColorConfig::Default } else { ClearColorConfig::None };
    main_interface.output_mode = CameraOutputMode::Write {
        blend_state: Some(BlendState::ALPHA_BLENDING),
        clear_color: main_clear,
    };
    if !floating {
        // Every camera drawing into the window (the 3D view's and the
        // position handle's) goes back to the main window as the window
        // goes: one left behind would look for the window that is gone.
        for (camera, target) in &cameras {
            if let RenderTarget::Window(WindowRef::Entity(window)) = target
                && windows.contains(*window)
            {
                commands.entity(camera).insert(RenderTarget::Window(WindowRef::Primary));
            }
        }
        for (window, _) in &windows {
            commands.entity(window).despawn();
        }
        for interface in &interfaces {
            commands.entity(interface).despawn();
        }
        return;
    }

    let leftmost = monitors.iter().min_by_key(|monitor| monitor.physical_position.x);
    let monitor_count = monitors.iter().count();
    let window = window_on_monitor(title(&editor), leftmost, monitor_count);
    let window_entity = commands.spawn((window, FloatingViewportWindow)).id();
    let output_mode = CameraOutputMode::Write {
        blend_state: Some(BlendState::ALPHA_BLENDING),
        clear_color: ClearColorConfig::None,
    };
    let camera = Camera {
        order: 2,
        output_mode,
        clear_color: ClearColorConfig::Custom(Color::NONE),
        ..default()
    };
    let target = RenderTarget::Window(WindowRef::Entity(window_entity));
    commands.spawn((FloatingViewportUi, Camera2d, RenderLayers::none(), camera, target.clone(), EguiSchedule::new(FloatingViewportPass)));
    commands.entity(*viewport_camera).insert(target);
}

/// The interface whose context has been styled: each time the window
/// opens, it opens with a new one.
#[derive(Default)]
pub struct Styled(Option<Entity>);

/// Draws the viewport area over the whole floating window and fits the
/// 3D view into it; what it asks for runs with the main window's
/// commands next frame.
#[allow(clippy::too_many_arguments)]
pub fn draw(
    context: Single<(Entity, &mut EguiContext), With<FloatingViewportUi>>,
    mut main_context: Single<&mut EguiContext, (With<PrimaryEguiContext>, Without<FloatingViewportUi>)>,
    editor: Res<Editor>,
    mut ui_state: ResMut<UiState>,
    mut settings: ResMut<ViewportSettings>,
    mut input: ResMut<ViewportInput>,
    navigation: Res<NavigationSettings>,
    camera: Single<(&mut Camera, &StrategyCamera), With<ViewportCamera>>,
    window: Single<&Window, With<FloatingViewportWindow>>,
    mut styled: Local<Styled>,
) {
    let (interface, mut context) = context.into_inner();
    let ctx = context.get_mut().clone();
    let main_ctx = main_context.get_mut().clone();
    if styled.0 != Some(interface) {
        theme::apply(&ctx);
        ctx.options_mut(|options| options.zoom_with_keyboard = false);
        egui_extras::install_image_loaders(&ctx);
        styled.0 = Some(interface);
        ctx.request_repaint();
        return;
    }
    ctx.set_zoom_factor(editor.user.options.ui_scale());
    crate::theme::set_settings(&ctx, editor.user.options.interface_settings());
    crate::ui::formulas::publish(&ctx, &editor);
    crate::ui::menu_keys::begin(&ctx);
    let lighting = viewport_area::lighting_panel_id();
    if ui_state.sun_open && tool_window::take_escape(&ctx, &[lighting]).is_some() {
        ui_state.sun_open = false;
    }
    let docked_keys_blocked = crate::ui::tool_commands::modal_open(&ctx, &ui_state);
    ui_state.viewport_2d.begin_keyboard(&ctx, docked_keys_blocked);
    let full = ctx.content_rect();
    let builder = UiBuilder::new().layer_id(LayerId::background()).max_rect(full);
    let mut root = Ui::new(ctx.clone(), "nerthus-floating-viewport".into(), builder);
    let background = root.painter().add(egui::Shape::Noop);

    let (mut camera, strategy) = camera.into_inner();
    let hint = viewport_area::navigation_hint_text(&navigation);
    let now = ctx.input(|state| state.time);
    let hint_shows = ui_state.navigation_hint_until.is_some_and(|until| now < until);
    let overlays = viewport_area::ViewOverlays {
        north_angle: strategy.north_angle(),
        navigation_hint: hint_shows.then_some(hint.as_str()),
    };
    let mut pending = Vec::new();
    let area = full.shrink(AREA_MARGIN);
    let interface = ui_state.as_mut();
    let host = viewport_area::show(&mut root, area, &editor, &mut settings, &mut interface.sun_open, &mut interface.viewport_2d, &overlays, &mut pending);
    crate::ui::viewport_2d::picked_dialog(&ctx, &mut interface.viewport_2d, &mut pending);
    tool_window::note_keyboard(&ctx);
    interface.viewport_2d.finish_keyboard(&ctx);
    root.painter().set(background, surround(full, host));
    interface.floating_commands.extend(pending);

    let physical_size = UVec2::new(window.physical_width(), window.physical_height());
    fit_camera(&mut camera, ctx.pixels_per_point(), physical_size, host);
    // The 3D view takes the pointer and keys here as it does embedded.
    let pointer = ctx.input(|state| state.pointer.hover_pos());
    let over_view = pointer.is_some_and(|pointer| host.contains(pointer));
    let layer = pointer.and_then(|pointer| ctx.layer_id_at(pointer));
    let over_window = layer.is_some_and(|layer| layer.order != egui::Order::Background);
    let dialog_open = crate::ui::tool_commands::modal_open(&ctx, interface);
    input.enabled = window.focused && over_view && !over_window && !dialog_open && !ctx.egui_is_using_pointer() && !ctx.any_popup_open();
    // Bevy's key state is shared by both windows. Losing this window's
    // OS focus must also release the camera, including when Alt+Tab is used.
    if !window.focused {
        interface.viewport_focused = false;
    } else if ctx.input(|state| state.pointer.any_pressed()) {
        interface.viewport_focused = over_view && !over_window;
    }
    // egui retains a field's focus while its native window is inactive.
    // Only the active main context can claim the shared keyboard.
    let main_focused = main_ctx.input(|state| state.focused);
    let main_keys_blocked = main_focused && (main_ctx.egui_wants_keyboard_input() || main_ctx.any_popup_open());
    let modifiers = ctx.input(|state| state.modifiers);
    input.keys = window.focused && interface.viewport_focused && !main_keys_blocked && !dialog_open && !ctx.egui_wants_keyboard_input() && !ctx.any_popup_open() && !modifiers.command && !modifiers.ctrl;
}

#[cfg(test)]
mod focus_tests {
    use super::*;

    #[test]
    fn floating_camera_uses_only_its_windows_keyboard() {
        let mut world = World::new();
        world.insert_resource(Editor::new(crate::document::Document::new()));
        world.insert_resource(UiState::default());
        world.init_resource::<ViewportSettings>();
        world.init_resource::<ViewportInput>();
        world.init_resource::<NavigationSettings>();

        let mut floating_context = EguiContext::default();
        let ctx = floating_context.get_mut().clone();
        world.spawn((floating_context, FloatingViewportUi));
        let mut main_context = EguiContext::default();
        let main_ctx = main_context.get_mut().clone();
        world.spawn((main_context, PrimaryEguiContext));
        world.spawn((Camera::default(), StrategyCamera::default(), ViewportCamera));
        let window = Window { resolution: (800, 600).into(), ..default() };
        let window_entity = world.spawn((window, FloatingViewportWindow)).id();
        let draw_system = world.register_system(draw);

        // The initial pass installs the theme, as it does in the app.
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0));
        let input = egui::RawInput { screen_rect: Some(screen), ..default() };
        ctx.begin_pass(input);
        world.run_system(draw_system).unwrap();
        ctx.end_pass().drop_without_applying_deltas();

        let mut frame = |window_focused, main_focused, modifiers| {
            world.get_mut::<Window>(window_entity).unwrap().focused = window_focused;
            world.resource_mut::<UiState>().viewport_focused = true;
            // Keep a stale hover over the floating view while the main
            // window receives typing, matching the original leak.
            let events = vec![
                egui::Event::PointerMoved(egui::pos2(100.0, 100.0)),
                egui::Event::ModifiersChanged(modifiers),
            ];
            let input = egui::RawInput { screen_rect: Some(screen), focused: window_focused, events, ..default() };
            ctx.begin_pass(input);
            let main_input = egui::RawInput { focused: main_focused, ..default() };
            main_ctx.begin_pass(main_input);
            main_ctx.memory_mut(|memory| memory.request_focus(egui::Id::new("main-text-field")));
            assert!(main_ctx.egui_wants_keyboard_input());

            world.run_system(draw_system).unwrap();
            let input = world.resource::<ViewportInput>();
            let result = (input.keys, input.enabled, world.resource::<UiState>().viewport_focused);
            ctx.end_pass().drop_without_applying_deltas();
            main_ctx.end_pass().drop_without_applying_deltas();
            result
        };

        // Main-window typing and focus switches without a pointer click
        // release both camera inputs and its higher frame-rate focus.
        assert_eq!(frame(false, true, egui::Modifiers::NONE), (false, false, false));
        // A retained main text focus cannot block the active viewport.
        assert!(frame(true, false, egui::Modifiers::NONE).0);
        // The main context also protects typing while native focus
        // changes are still being reflected between the two passes.
        assert!(!frame(true, true, egui::Modifiers::NONE).0);
        assert!(!frame(true, false, egui::Modifiers::CTRL).0);
        assert!(!frame(true, false, egui::Modifiers::COMMAND).0);
    }
}

#[cfg(test)]
mod window_tests {
    use super::*;

    fn monitor(width: u32, height: u32, scale_factor: f64) -> Monitor {
        Monitor {
            name: None,
            physical_width: width,
            physical_height: height,
            physical_position: IVec2::new(-1920, 0),
            refresh_rate_millihertz: None,
            scale_factor,
            video_modes: Vec::new(),
        }
    }

    #[test]
    fn small_and_scaled_screens_get_a_fitting_window_and_restore_size() {
        let screens = [(1366, 768, 1.0), (1920, 1080, 1.25), (1920, 1080, 1.5), (1280, 800, 1.0)];
        for (width, height, scale_factor) in screens {
            let monitor = monitor(width, height, scale_factor);
            let mut window = window_on_monitor("Nerthus".to_string(), Some(&monitor), 1);
            assert_eq!(window.internal.take_maximize_request(), Some(true));
            assert_eq!(window.position, WindowPosition::At(monitor.physical_position));

            let logical_size = monitor.physical_size().as_vec2() / scale_factor as f32;
            let restored_size = window.resolution.size() + WINDOW_FRAME_ALLOWANCE;
            assert!(restored_size.x <= logical_size.x);
            assert!(restored_size.y <= logical_size.y);
        }
    }

    #[test]
    fn large_screens_keep_the_normal_size_and_multiple_screens_maximize() {
        let monitor = monitor(3840, 2160, 2.0);
        for monitor_count in [1, 2] {
            let mut window = window_on_monitor("Nerthus".to_string(), Some(&monitor), monitor_count);
            assert_eq!(window.resolution.physical_size(), UVec2::new(1280, 800));
            let expected = (monitor_count > 1).then_some(true);
            assert_eq!(window.internal.take_maximize_request(), expected);
        }
    }

    #[test]
    fn an_unknown_monitor_leaves_fitting_to_the_window_manager() {
        let mut window = window_on_monitor("Nerthus".to_string(), None, 0);
        assert_eq!(window.internal.take_maximize_request(), Some(true));
    }
}
