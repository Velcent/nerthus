//! Position AppKit's own window controls within the custom title bar.

use objc2::MainThreadMarker;
use objc2::rc::Retained;
use objc2_app_kit::{NSView, NSWindowButton, NSWindowStyleMask};
use objc2_foundation::{NSPoint, NSRect, NSSize};
use raw_window_handle::RawWindowHandle;

/// Position the close button's left edge and all three buttons' vertical
/// center in Cocoa points from the content view's top left. Their native
/// sizes, horizontal spacing, drawing and actions stay with AppKit.
/// `bar_height` is the app title bar's height in the same Cocoa points.
///
/// Call each UI frame: AppKit restores its own layout during resizing and
/// fullscreen transitions. Positions are absolute so repeated calls do not
/// accumulate offsets, and interface zoom does not move the controls.
pub fn position_traffic_lights(
    handles: &bevy::window::RawHandleWrapper,
    left_inset: f64,
    center_y: f64,
    bar_height: f64,
) {
    if MainThreadMarker::new().is_none()
        || !left_inset.is_finite()
        || !center_y.is_finite()
        || !bar_height.is_finite()
    {
        return;
    }
    if left_inset < 0.0 || center_y <= 0.0 || bar_height <= 0.0 {
        return;
    }
    let RawWindowHandle::AppKit(handle) = handles.get_window_handle() else {
        return;
    };
    // SAFETY: The caller holds Bevy's live window handle on the main
    // thread; AppKit owns the view for the duration of this call.
    let content_view: &NSView = unsafe { handle.ns_view.cast::<NSView>().as_ref() };
    let Some(window) = content_view.window() else {
        return;
    };
    if window.styleMask().contains(NSWindowStyleMask::FullScreen) {
        return;
    }
    let content_bounds = content_view.bounds();
    let native_height = content_bounds.size.height - window.contentLayoutRect().size.height;
    // Hidden fullscreen title bars can report zero before the style mask
    // changes. Let AppKit own the controls throughout that transition.
    if !(10.0..=80.0).contains(&native_height) {
        return;
    }
    let Some(close) = window.standardWindowButton(NSWindowButton::CloseButton) else {
        return;
    };
    let Some(minimize) = window.standardWindowButton(NSWindowButton::MiniaturizeButton) else {
        return;
    };
    let Some(zoom) = window.standardWindowButton(NSWindowButton::ZoomButton) else {
        return;
    };
    // SAFETY: AppKit owns this native window hierarchy, and the main
    // thread is its only reader and writer throughout this call.
    let Some(button_parent) = (unsafe { close.superview() }) else {
        return;
    };
    let Some(window_content) = window.contentView() else {
        return;
    };
    // SAFETY: The content view belongs to the same live AppKit window.
    let Some(window_frame) = (unsafe { window_content.superview() }) else {
        return;
    };

    // Grow the enclosing native title bar as well as the buttons' parent.
    // NSView hit testing clips to ancestor bounds, so moving only button
    // frames would leave lower or horizontally inset controls unclickable.
    // Find those ancestors through the public hierarchy, without depending
    // on AppKit's private title bar class names.
    let mut ancestors = Vec::new();
    let mut ancestor = Retained::clone(&button_parent);
    while !std::ptr::eq(&*ancestor, &*window_frame) {
        if std::ptr::eq(&*ancestor, &*window_content) {
            return;
        }
        // SAFETY: This retained view is in the live native hierarchy.
        let Some(parent) = (unsafe { ancestor.superview() }) else {
            return;
        };
        ancestors.push(ancestor);
        ancestor = parent;
    }
    if ancestors.is_empty() {
        return;
    }

    let close_frame = close.frame();
    let buttons = [close, minimize, zoom];
    let button_frames = buttons.each_ref().map(|button| button.frame());
    let mut max_button_height = 0.0_f64;
    for frame in &button_frames {
        max_button_height = max_button_height.max(frame.size.height);
    }
    let buttons_bottom = center_y + max_button_height / 2.0;
    let bar_height = native_height
        .max(bar_height)
        .max(buttons_bottom)
        .min(content_bounds.size.height);
    let bar_y = if content_view.isFlipped() {
        content_bounds.origin.y
    } else {
        content_bounds.origin.y + content_bounds.size.height - bar_height
    };
    let bar_origin = NSPoint::new(content_bounds.origin.x, bar_y);
    let bar_size = NSSize::new(content_bounds.size.width, bar_height);
    let bar_rect = NSRect::new(bar_origin, bar_size);
    for ancestor in ancestors.iter().rev() {
        // SAFETY: This retained view is in the live native hierarchy.
        let Some(parent) = (unsafe { ancestor.superview() }) else {
            return;
        };
        let frame = parent.convertRect_fromView(bar_rect, Some(content_view));
        if ancestor.frame() != frame {
            ancestor.setFrame(frame);
        }
    }

    let center_y = if content_view.isFlipped() {
        content_bounds.origin.y + center_y
    } else {
        content_bounds.origin.y + content_bounds.size.height - center_y
    };
    for (button, original_frame) in buttons.iter().zip(button_frames) {
        // SAFETY: AppKit owns this live native button and its parent.
        let Some(parent) = (unsafe { button.superview() }) else {
            continue;
        };
        let horizontal_offset = original_frame.origin.x - close_frame.origin.x;
        let center_x = content_bounds.origin.x
            + left_inset
            + horizontal_offset
            + original_frame.size.width / 2.0;
        let content_center = NSPoint::new(center_x, center_y);
        let parent_center = parent.convertPoint_fromView(content_center, Some(content_view));
        let origin = NSPoint::new(
            parent_center.x - original_frame.size.width / 2.0,
            parent_center.y - original_frame.size.height / 2.0,
        );
        if button.frame().origin != origin {
            button.setFrameOrigin(origin);
        }
    }
}
