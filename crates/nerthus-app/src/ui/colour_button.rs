//! `ColorPicker`: a button showing a colour, which opens the
//! `MinimalColorPicker` under it (a saturation and brightness square over
//! a hue strip). As a colour property's `ColorPickerEditor`, a right-click
//! on the button types a colour in an `EditablePopup`; a right-click on the
//! picker does so for the picker. Colours go in and come out as WPF's sRGB
//! bytes, the picker keeping its hue, saturation and value between them.

use egui::{Color32, Id, Pos2, Rect, Sense, Shape, Stroke, StrokeKind, Ui, Vec2};

use crate::theme;
use crate::ui::colour_picker::{linear_to_srgb, srgb_to_linear};
use crate::ui::slider_popups::{self, BoxWidth, TypingLook};

/// The button's padding (`Padding="24,12"`).
const PADDING: Vec2 = Vec2::new(24.0, 12.0);
/// The system theme's button border (`Button.Static.Border`), which
/// `Button-Template` draws round the colour.
const BORDER: Color32 = Color32::from_rgb(0x70, 0x70, 0x70);
/// `Popup.VerticalOffset`: the picker comes up over the button's foot.
const POPUP_OFFSET: f32 = -10.0;
/// The picker (at 100%): its least size (`MinWidth`, `MinHeight`), its
/// grid's margin, the gap over the hue strip and the strip's height.
const PICKER_MIN_SIZE: f32 = 200.0;
const PICKER_MARGIN: f32 = 10.0;
const HUE_GAP: f32 = 8.0;
const HUE_HEIGHT: f32 = 40.0;
/// The cursor ring (14 across) and the hue marker (14 wide), each drawn in
/// a 2 line with a shadow 5 round it.
const MARK_SIZE: f32 = 14.0;
const MARK_STROKE: f32 = 2.0;
const MARK_SHADOW: f32 = 5.0;
/// The typing boxes: monospace 16, 16 wider than what they are over and
/// 64 at least.
const TYPING_FONT_SIZE: f32 = 16.0;
const TYPING_EXTRA_WIDTH: f32 = 16.0;
const TYPING_MIN_WIDTH: f32 = 64.0;

/// What a `ColorPicker` keeps between frames.
#[derive(Clone, Default)]
struct PickerState {
    /// The colour last set from outside or given out, which the rest
    /// follows.
    synced: Option<[u8; 3]>,
    /// `CurrentColor`.
    colour: [u8; 3],
    /// `_hue` (degrees), `_sat` and `_val`.
    hue: f32,
    saturation: f32,
    value: f32,
    /// The cursor ring and the hue marker, as fractions of the square and
    /// of the strip: a pick puts them under the pointer, past an edge too.
    cursor: Vec2,
    marker: f32,
    open: bool,
}

impl PickerState {
    /// `SetColor`: the hue, saturation and value of `rgb`, the marks at
    /// them, and `CurrentColor` made from them again.
    fn set_colour(&mut self, rgb: [u8; 3]) {
        let [hue, saturation, value] = rgb_to_hsv(rgb);
        self.hue = hue;
        self.saturation = saturation;
        self.value = value;
        self.cursor = Vec2::new(saturation, 1.0 - value);
        self.marker = hue / 360.0;
        self.update_colour();
    }

    /// `UpdateColor`.
    fn update_colour(&mut self) {
        self.colour = hsv_to_rgb(self.hue, self.saturation, self.value);
    }
}

/// `MinimalColorPicker.RGBtoHSV`: the hue in degrees.
fn rgb_to_hsv(rgb: [u8; 3]) -> [f32; 3] {
    let [red, green, blue] = rgb.map(|byte| byte as f32 / 255.0);
    let maximum = red.max(green.max(blue));
    let minimum = red.min(green.min(blue));
    let delta = maximum - minimum;
    let mut hue = 0.0;
    if delta != 0.0 {
        hue = if maximum == red {
            60.0 * ((green - blue) / delta % 6.0)
        } else if maximum == green {
            60.0 * ((blue - red) / delta + 2.0)
        } else {
            60.0 * ((red - green) / delta + 4.0)
        };
    }
    if hue < 0.0 {
        hue += 360.0;
    }
    let saturation = if maximum == 0.0 { 0.0 } else { delta / maximum };
    [hue, saturation, maximum]
}

/// `MinimalColorPicker.HSVtoRGB`: the bytes cut short, not rounded.
fn hsv_to_rgb(hue: f32, saturation: f32, value: f32) -> [u8; 3] {
    let chroma = value * saturation;
    let second = chroma * (1.0 - (hue / 60.0 % 2.0 - 1.0).abs());
    let offset = value - chroma;
    let (red, green, blue) = if hue < 60.0 {
        (chroma, second, 0.0)
    } else if hue < 120.0 {
        (second, chroma, 0.0)
    } else if hue < 180.0 {
        (0.0, chroma, second)
    } else if hue < 240.0 {
        (0.0, second, chroma)
    } else if hue < 300.0 {
        (second, 0.0, chroma)
    } else {
        (chroma, 0.0, second)
    };
    [red, green, blue].map(|component| ((component + offset) * 255.0) as u8)
}

/// The text's colour on a button showing `colour` (`Mcp_OnColorUpdate`):
/// one less the bytes' mean, as an scRGB grey, so black on all but the
/// darkest colours.
fn text_colour(colour: [u8; 3]) -> Color32 {
    let sum = colour[0] as f32 + colour[1] as f32 + colour[2] as f32;
    Color32::from_gray(linear_to_srgb(1.0 - sum / 3.0))
}

/// A colour's floats as Gaea writes them (`Color.ToString`, four places)
/// with commas between, as the typing boxes start from.
fn colour_text(floats: [f32; 3]) -> String {
    let texts = floats.map(|component| format!("{component:.4}"));
    texts.join(", ")
}

/// `new Color(text)` for a text with a `#`: the two hex digits after its
/// first character, the two after those and the two after those, each
/// over 255; black when any of them is missing or not hex.
fn hex_colour(text: &str) -> [f32; 3] {
    let characters: Vec<char> = text.chars().collect();
    let pair = |start: usize| {
        let digits: String = characters.get(start..start + 2)?.iter().collect();
        let byte = u8::from_str_radix(digits.trim(), 16).ok()?;
        Some(byte as f32 / 255.0)
    };
    match (pair(1), pair(3), pair(5)) {
        (Some(red), Some(green), Some(blue)) => [red, green, blue],
        _ => [0.0; 3],
    }
}

/// `Single.TryParse` and `Byte.TryParse`: white space round the number
/// allowed.
fn parse_float(text: &str) -> Option<f32> {
    text.trim().parse().ok()
}

fn parse_byte(text: &str) -> Option<f32> {
    text.trim().parse::<u8>().ok().map(|byte| byte as f32 / 255.0)
}

/// What `ColorPickerEditor.EditablePopup_EditSuccess` sets a property
/// that holds `current` to for `text`: `#RRGGBB`, or three numbers split
/// by commas (one for a grey), as floats when there is a point or every
/// part is just 0 or 1, and otherwise as bytes over 255. What does not
/// parse leaves `current`.
fn typed_property_colour(text: &str, current: [f32; 3]) -> [f32; 3] {
    if text.contains('#') {
        return hex_colour(text);
    }
    let parts: Vec<&str> = text.split(',').collect();
    let floats = text.contains('.') || parts.iter().all(|part| *part == "0" || *part == "1");
    let parse = if floats { parse_float } else { parse_byte };
    if parts.len() == 3 {
        let components: Option<Vec<f32>> = parts.iter().map(|part| parse(part)).collect();
        return components.map_or(current, |components| [components[0], components[1], components[2]]);
    }
    parse(text).map_or(current, |grey| [grey; 3])
}

/// What `MinimalColorPicker.EditablePopup_EditSuccess` makes of `text`:
/// `#RRGGBB`, or three numbers split by commas (one for a grey), as floats
/// when there is a point and otherwise as bytes over 255; None when they
/// do not parse.
fn typed_picker_colour(text: &str) -> Option<[f32; 3]> {
    if text.contains('#') {
        return Some(hex_colour(text));
    }
    let parts: Vec<&str> = text.split(',').collect();
    let parse = if text.contains('.') { parse_float } else { parse_byte };
    if parts.len() == 3 {
        let components: Option<Vec<f32>> = parts.iter().map(|part| parse(part)).collect();
        return components.map(|components| [components[0], components[1], components[2]]);
    }
    parse(text).map(|grey| [grey; 3])
}

/// How high a `ColorPicker` with text in it is: a line of the text in its
/// padding.
pub fn button_height(ui: &Ui) -> f32 {
    let line = ui.fonts_mut(|fonts| fonts.row_height(&theme::interface(12.0)));
    line + 2.0 * PADDING.y
}

/// How wide and high a `ColorPicker` with nothing in it is: its padding.
pub fn empty_button_size() -> Vec2 {
    2.0 * PADDING
}

/// A `ColorPicker` over `rect` showing `rgb`, with `text` centred in it
/// (`Content`). A click opens the `MinimalColorPicker` under it; with
/// `typed_from`, a colour property's floats, a right-click types a colour
/// over it (`ColorPickerEditor`), and the property then takes what the
/// picker makes of that. Returns the colour picked or typed
/// (`OnColorUpdate`), which the caller keeps as `ToColor` does.
pub fn colour_button(ui: &mut Ui, id: Id, rect: Rect, rgb: [u8; 3], text: &str, typed_from: Option<[f32; 3]>) -> Option<[u8; 3]> {
    let mut state: PickerState = ui.data(|data| data.get_temp(id)).unwrap_or_default();
    if state.synced != Some(rgb) {
        state.set_colour(rgb);
        state.synced = Some(rgb);
    }
    let response = ui.interact(rect, id.with("button"), Sense::click());
    paint_button(ui, rect, state.colour, text);
    if response.clicked() {
        state.open = true;
    }
    let scale = slider_popups::popup_scale(ui.ctx());
    let mut picked = None;

    // `ColorPickerEditor.OnMouseRightButtonUp`: the property's colour as
    // text, in a box centred over the button.
    let button_typing = id.with("button-typing");
    if let Some(floats) = typed_from {
        if response.secondary_clicked() {
            slider_popups::open_typing(ui.ctx(), button_typing, colour_text(floats));
        }
        let width = ((rect.width() + TYPING_EXTRA_WIDTH) * scale).max(TYPING_MIN_WIDTH * scale);
        let look = centred_typing(ui, rect.center(), width);
        if let Some(typed) = slider_popups::typing_box(ui, button_typing, &look) {
            // Then, as it always does (`finally`), `SetColor` with what the
            // property holds, which sets the property in turn.
            let property = typed_property_colour(&typed, floats);
            state.set_colour(property.map(linear_to_srgb));
            picked = Some(state.colour);
        }
    }

    let picker = if state.open { Some(picker_popup(ui, id, rect, &mut state)) } else { None };
    if let Some(picker) = picker {
        picked = picker.picked.or(picked);
        // `MinimalColorPicker_MouseRightButtonUp`: the picker's colour as
        // text, in a box centred over it.
        if picker.typing_asked {
            slider_popups::open_typing(ui.ctx(), id.with("picker-typing"), colour_text(state.colour.map(srgb_to_linear)));
        }
        state.open = !picker.closed;
        ui.data_mut(|data| data.insert_temp(id.with("picker-frame"), picker.frame));
    }
    // The picker's box outlives the picker: a press in it closes the
    // picker, but the colour typed still goes to it.
    let picker_typing = id.with("picker-typing");
    if slider_popups::typing_open(ui.ctx(), picker_typing)
        && let Some(frame) = ui.data(|data| data.get_temp::<Rect>(id.with("picker-frame")))
    {
        let width = (frame.width() + TYPING_EXTRA_WIDTH * scale).max(TYPING_MIN_WIDTH * scale);
        let look = centred_typing(ui, frame.center(), width);
        if let Some(typed) = slider_popups::typing_box(ui, picker_typing, &look)
            && let Some(floats) = typed_picker_colour(&typed)
        {
            state.set_colour(floats.map(linear_to_srgb));
            picked = Some(state.colour);
        }
    }

    if picked.is_some() {
        state.synced = Some(state.colour);
    }
    ui.data_mut(|data| data.insert_temp(id, state));
    picked
}

/// A typing box `width` wide centred on `centre` (`PlacementMode.Center`).
fn centred_typing(ui: &Ui, centre: Pos2, width: f32) -> TypingLook {
    let font = theme::monospace(TYPING_FONT_SIZE);
    let height = slider_popups::typing_height(ui, &font);
    let top_left = centre - Vec2::new(width, height) / 2.0;
    TypingLook { top_left, width: BoxWidth::Fixed(width), font }
}

/// The button (`Button-Template`): the colour inside the theme's border
/// (corners 2), `text` centred in it in the colour that stands out on it;
/// disabled, `Button-Disabled-Background` and the text at half.
fn paint_button(ui: &Ui, rect: Rect, colour: [u8; 3], text: &str) {
    let enabled = ui.is_enabled();
    let fill = if enabled { Color32::from_rgb(colour[0], colour[1], colour[2]) } else { theme::BUTTON_DISABLED_BACKGROUND };
    ui.painter().rect_filled(rect, 2.0, fill);
    ui.painter().rect_stroke(rect, 2.0, Stroke::new(1.0, BORDER), StrokeKind::Inside);
    if text.is_empty() {
        return;
    }
    let mut colour = text_colour(colour);
    if !enabled {
        colour = colour.gamma_multiply(0.5);
    }
    let galley = ui.painter().layout_no_wrap(text.to_string(), theme::interface(12.0), colour);
    let content = rect.shrink2(PADDING);
    let position = content.center() - galley.size() / 2.0;
    let painter = ui.painter().with_clip_rect(ui.clip_rect().intersect(rect));
    painter.galley(position, galley, colour);
}

/// What the picker did this frame.
struct PickerOutcome {
    /// Where it shows.
    frame: Rect,
    picked: Option<[u8; 3]>,
    typing_asked: bool,
    closed: bool,
}

/// The `MinimalColorPicker` in its popup under `button`: a square as wide
/// as the button and 200 (at 100%) at least, on the dark window colour
/// (corners 4); inside its margin of 10, the saturation (left to right)
/// and brightness (top to bottom) square at the hue, and 8 below it the
/// hue strip (40 high). Pressing or moving with the button down over
/// either picks, as long as the pointer is over it (nothing holds it); a
/// right-click asks for the typing box, and a press outside closes the
/// picker (`StaysOpen` off).
fn picker_popup(ui: &Ui, id: Id, button: Rect, state: &mut PickerState) -> PickerOutcome {
    let scale = slider_popups::popup_scale(ui.ctx());
    let size = button.width().max(PICKER_MIN_SIZE * scale);
    let screen = ui.ctx().content_rect();
    // `PlacementMode.Bottom`, 10 up over the button; above it when there
    // is no room below, and kept in the window.
    let mut top = button.bottom() + POPUP_OFFSET;
    if top + size > screen.bottom() {
        top = button.top() - POPUP_OFFSET - size;
    }
    let top = top.min(screen.bottom() - size).max(screen.top());
    let left = button.left().min(screen.right() - size).max(screen.left());
    let frame = Rect::from_min_size(Pos2::new(left, top), Vec2::splat(size));

    let mut picked = None;
    let area = egui::Area::new(id.with("picker")).fixed_pos(frame.min).constrain(false).order(egui::Order::Foreground);
    let shown = area.show(ui.ctx(), |ui| {
        let (rect, response) = ui.allocate_exact_size(frame.size(), Sense::click());
        ui.painter().rect_filled(rect, 4.0 * scale, theme::WINDOW_DARK);
        let inner = rect.shrink(PICKER_MARGIN * scale);
        let strip_height = HUE_HEIGHT * scale;
        let square_bottom = inner.bottom() - strip_height - HUE_GAP * scale;
        let square = Rect::from_min_max(inner.min, Pos2::new(inner.right(), square_bottom));
        let strip = Rect::from_min_max(Pos2::new(inner.left(), inner.bottom() - strip_height), inner.max);

        // A press, or a move with the button down, over a canvas picks.
        let (down, acted, pointer) = ui.input(|input| {
            let acted = input.pointer.primary_pressed() || input.pointer.delta() != Vec2::ZERO;
            (input.pointer.primary_down(), acted, input.pointer.latest_pos())
        });
        if let Some(pointer) = pointer.filter(|_| down && acted) {
            if square.contains(pointer) {
                // `PickColor`.
                let at = (pointer - square.min) / square.size();
                state.saturation = at.x.clamp(0.0, 1.0);
                state.value = (1.0 - at.y).clamp(0.0, 1.0);
                state.cursor = at;
                state.update_colour();
                picked = Some(state.colour);
            } else if strip.contains(pointer) {
                // `UpdateHueFromPosition`.
                let along = (pointer.x - strip.left()) / strip.width();
                state.hue = along.clamp(0.0, 1.0) * 360.0;
                state.marker = along;
                state.update_colour();
                picked = Some(state.colour);
            }
        }

        paint_square(ui, square, state.hue);
        paint_hue_strip(ui, strip);
        // The marks, cut off at their canvases' edges.
        let cursor_centre = square.min + state.cursor * square.size();
        let cursor_painter = ui.painter().with_clip_rect(ui.clip_rect().intersect(square));
        let ring_radius = (MARK_SIZE - MARK_STROKE) / 2.0 * scale;
        cursor_painter.circle_stroke(cursor_centre, ring_radius, Stroke::new((MARK_STROKE + MARK_SHADOW) * scale, Color32::from_black_alpha(80)));
        cursor_painter.circle_stroke(cursor_centre, ring_radius, Stroke::new(MARK_STROKE * scale, Color32::WHITE));
        let marker_left = strip.left() + state.marker * strip.width() - MARK_SIZE / 2.0 * scale;
        let marker = Rect::from_min_size(Pos2::new(marker_left, strip.top()), Vec2::new(MARK_SIZE * scale, strip.height()));
        let marker_painter = ui.painter().with_clip_rect(ui.clip_rect().intersect(strip));
        marker_painter.rect_stroke(marker, 0.0, Stroke::new(MARK_SHADOW * scale, Color32::from_black_alpha(80)), StrokeKind::Middle);
        marker_painter.rect_stroke(marker, 0.0, Stroke::new(MARK_STROKE * scale, Color32::WHITE), StrokeKind::Inside);
        response.secondary_clicked()
    });
    let pressed_outside = ui.input(|input| input.pointer.any_pressed() && input.pointer.press_origin().is_some_and(|origin| !frame.contains(origin)));
    PickerOutcome { frame, picked, typing_asked: shown.inner, closed: pressed_outside }
}

/// The saturation and brightness square: `currentcolor` (the hue at full
/// saturation and value), white fading out to the right over it and black
/// coming in towards the foot, blended as WPF blends them (in sRGB): each
/// channel is (1 - y) * (1 - x + hue * x).
fn paint_square(ui: &Ui, rect: Rect, hue: f32) {
    let base = hsv_to_rgb(hue, 1.0, 1.0).map(|byte| byte as f32 / 255.0);
    let columns = 24;
    let rows = 16;
    let mut mesh = egui::Mesh::default();
    for row in 0..=rows {
        for column in 0..=columns {
            let x = column as f32 / columns as f32;
            let y = row as f32 / rows as f32;
            let channel = |component: f32| ((1.0 - y) * (1.0 - x + component * x) * 255.0).round() as u8;
            let colour = Color32::from_rgb(channel(base[0]), channel(base[1]), channel(base[2]));
            mesh.colored_vertex(rect.min + Vec2::new(x, y) * rect.size(), colour);
        }
    }
    let stride = columns + 1;
    for row in 0..rows {
        for column in 0..columns {
            let corner = (row * stride + column) as u32;
            let below = corner + stride as u32;
            mesh.add_triangle(corner, corner + 1, below);
            mesh.add_triangle(corner + 1, below + 1, below);
        }
    }
    ui.painter().add(Shape::mesh(mesh));
}

/// The hue strip's gradient stops, red round to red, left to right.
const HUE_STOPS: [(f32, [u8; 3]); 7] = [
    (0.0, [0xFF, 0x00, 0x00]),
    (0.167, [0xFF, 0xFF, 0x00]),
    (0.333, [0x00, 0xFF, 0x00]),
    (0.5, [0x00, 0xFF, 0xFF]),
    (0.667, [0x00, 0x00, 0xFF]),
    (0.833, [0xFF, 0x00, 0xFF]),
    (1.0, [0xFF, 0x00, 0x00]),
];

/// The hue strip (`HueSlider`'s gradient, blended in sRGB between its
/// stops).
fn paint_hue_strip(ui: &Ui, rect: Rect) {
    let steps_between = 8;
    let mut mesh = egui::Mesh::default();
    for pair in HUE_STOPS.windows(2) {
        let (from, from_colour) = pair[0];
        let (to, to_colour) = pair[1];
        for step in 0..=steps_between {
            let along = step as f32 / steps_between as f32;
            let channel = |index: usize| (from_colour[index] as f32 + (to_colour[index] as f32 - from_colour[index] as f32) * along).round() as u8;
            let colour = Color32::from_rgb(channel(0), channel(1), channel(2));
            let x = rect.left() + (from + (to - from) * along) * rect.width();
            let first = mesh.vertices.len() as u32;
            mesh.colored_vertex(Pos2::new(x, rect.top()), colour);
            mesh.colored_vertex(Pos2::new(x, rect.bottom()), colour);
            if step > 0 {
                mesh.add_triangle(first - 2, first - 1, first);
                mesh.add_triangle(first - 1, first + 1, first);
            }
        }
    }
    ui.painter().add(Shape::mesh(mesh));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hsv_goes_as_gaea_s_and_cuts_its_bytes_short() {
        assert_eq!(rgb_to_hsv([255, 0, 0]), [0.0, 1.0, 1.0]);
        assert_eq!(rgb_to_hsv([0, 0, 0]), [0.0, 0.0, 0.0]);
        // A hue below red's (magenta to red) wraps round.
        let [hue, _, _] = rgb_to_hsv([255, 0, 128]);
        assert!(hue > 300.0 && hue < 360.0, "{hue}");
        assert_eq!(hsv_to_rgb(0.0, 1.0, 1.0), [255, 0, 0]);
        assert_eq!(hsv_to_rgb(240.0, 1.0, 1.0), [0, 0, 255]);
        // Cut short, not rounded: half of 255 is 127.
        assert_eq!(hsv_to_rgb(0.0, 0.0, 0.5), [127, 127, 127]);
        // A round trip can lose a step.
        let round_trip = |rgb: [u8; 3]| {
            let [hue, saturation, value] = rgb_to_hsv(rgb);
            hsv_to_rgb(hue, saturation, value)
        };
        assert_eq!(round_trip([255, 0, 0]), [255, 0, 0]);
        for rgb in [[242, 244, 247], [12, 200, 99], [128, 128, 128]] {
            let back = round_trip(rgb);
            assert!(back.iter().zip(rgb).all(|(back, rgb)| (rgb as i32 - *back as i32).abs() <= 1), "{rgb:?} came back as {back:?}");
        }
    }

    #[test]
    fn typed_colours_read_as_gaea_reads_them() {
        let current = [0.1, 0.2, 0.3];
        // Hex and bytes go over 255 as they are, no sRGB in between.
        assert_eq!(typed_property_colour("#FF0080", current), [1.0, 0.0, 128.0 / 255.0]);
        assert_eq!(typed_property_colour("255, 0, 51", current), [1.0, 0.0, 0.2]);
        assert_eq!(typed_property_colour("0.5, 0.25, 1", current), [0.5, 0.25, 1.0]);
        // All 0s and 1s are floats, but only when nothing pads them.
        assert_eq!(typed_property_colour("1,0,1", current), [1.0, 0.0, 1.0]);
        assert_eq!(typed_property_colour("1, 0, 1", current), [1.0 / 255.0, 0.0, 1.0 / 255.0]);
        // One number is a grey.
        assert_eq!(typed_property_colour("0.5", current), [0.5; 3]);
        assert_eq!(typed_property_colour("51", current), [0.2; 3]);
        // A bad hex is black; what does not parse leaves the colour.
        assert_eq!(typed_property_colour("#12", current), [0.0; 3]);
        assert_eq!(typed_property_colour("red", current), current);
        assert_eq!(typed_property_colour("300, 0, 0", current), current);
        // The picker's own box has no rule for 0s and 1s.
        assert_eq!(typed_picker_colour("1,0,1"), Some([1.0 / 255.0, 0.0, 1.0 / 255.0]));
        assert_eq!(typed_picker_colour("0.5, 0.5, 0.5"), Some([0.5; 3]));
        assert_eq!(typed_picker_colour("red"), None);
        assert_eq!(colour_text([0.8879231, 0.9046612, 0.9301109]), "0.8879, 0.9047, 0.9301");
    }

    #[test]
    fn the_text_is_black_on_all_but_the_darkest_colours() {
        assert_eq!(text_colour([0, 0, 0]), Color32::WHITE);
        assert_eq!(text_colour([3, 0, 0]), Color32::BLACK);
        assert_eq!(text_colour([242, 244, 247]), Color32::BLACK);
        let dim = text_colour([1, 0, 0]);
        assert!(dim.r() > 0 && dim.r() < 255, "{dim:?}");
    }

    const SCREEN: Rect = Rect::from_min_max(Pos2::ZERO, Pos2::new(600.0, 500.0));
    const BUTTON: Rect = Rect::from_min_max(Pos2::new(20.0, 20.0), Pos2::new(300.0, 60.0));

    fn themed_context() -> egui::Context {
        let context = egui::Context::default();
        theme::apply(&context);
        let output = context.run_ui(egui::RawInput::default(), |_| {});
        output.drop_without_applying_deltas();
        context
    }

    /// One frame of a button for `rgb` with these events: the colour it
    /// gave and the shapes drawn.
    fn frame(context: &egui::Context, rgb: [u8; 3], typed_from: Option<[f32; 3]>, events: Vec<egui::Event>) -> (Option<[u8; 3]>, Vec<egui::epaint::ClippedShape>) {
        let input = egui::RawInput { screen_rect: Some(SCREEN), events, ..Default::default() };
        let mut picked = None;
        let mut output = context.run_ui(input, |ui| {
            picked = colour_button(ui, Id::new("colour-test"), BUTTON, rgb, "Color", typed_from);
        });
        let shapes = std::mem::take(&mut output.shapes);
        output.drop_without_applying_deltas();
        (picked, shapes)
    }

    fn click(context: &egui::Context, rgb: [u8; 3], position: Pos2, button: egui::PointerButton) -> Vec<Option<[u8; 3]>> {
        let event = |pressed: bool| egui::Event::PointerButton { pos: position, button, pressed, modifiers: egui::Modifiers::NONE };
        let events = [vec![egui::Event::PointerMoved(position)], vec![event(true)], vec![event(false)]];
        events.into_iter().map(|events| frame(context, rgb, None, events).0).collect()
    }

    fn picker_frame(context: &egui::Context) -> Option<Rect> {
        context.memory(|memory| memory.area_rect(Id::new("colour-test").with("picker")))
    }

    #[test]
    fn the_button_shows_the_colour_and_centres_its_name() {
        let context = themed_context();
        let rgb = [242, 244, 247];
        let (_, shapes) = frame(&context, rgb, None, Vec::new());
        // `Background` is the picker's `CurrentColor`: the colour through
        // its hue, saturation and value.
        let [hue, saturation, value] = rgb_to_hsv(rgb);
        let [red, green, blue] = hsv_to_rgb(hue, saturation, value);
        let fill = shapes.iter().find_map(|clipped| match &clipped.shape {
            Shape::Rect(rect) if rect.rect == BUTTON && rect.fill != Color32::TRANSPARENT => Some(rect.fill),
            _ => None,
        });
        assert_eq!(fill, Some(Color32::from_rgb(red, green, blue)));
        let name = shapes.iter().find_map(|clipped| match &clipped.shape {
            Shape::Text(text) if text.galley.text() == "Color" => Some((text.pos + text.galley.size() / 2.0, text.fallback_color)),
            _ => None,
        });
        let (centre, colour) = name.expect("the name shows");
        assert!(centre.distance(BUTTON.center()) < 0.5, "{centre:?}");
        assert_eq!(colour, Color32::BLACK);
    }

    #[test]
    fn the_picker_comes_up_under_the_button_and_picks_until_a_press_outside() {
        let context = themed_context();
        let grey = [128, 128, 128];
        click(&context, grey, BUTTON.center(), egui::PointerButton::Primary);
        frame(&context, grey, None, Vec::new());
        let picker = picker_frame(&context).expect("a click opens the picker");
        // As wide as the button (280), its top 10 up over the button's foot.
        assert!((picker.width() - BUTTON.width()).abs() < 0.5 && (picker.height() - BUTTON.width()).abs() < 0.5, "{picker:?}");
        assert!((picker.top() - (BUTTON.bottom() - 10.0)).abs() < 0.5 && (picker.left() - BUTTON.left()).abs() < 0.5, "{picker:?}");
        // The square's right edge at its top: full saturation and value,
        // at the grey's hue (red).
        let square_top_right = picker.min + Vec2::new(picker.width() - PICKER_MARGIN - 1.0, PICKER_MARGIN + 1.0);
        let picked = click(&context, grey, square_top_right, egui::PointerButton::Primary);
        let picked = picked.into_iter().flatten().next().expect("a press on the square picks");
        assert!(picked[0] >= 250 && picked[1] <= 5 && picked[2] <= 5, "{picked:?}");
        // A third of the way along the strip: green's hue.
        let strip = picker.min + Vec2::new(PICKER_MARGIN + (picker.width() - 2.0 * PICKER_MARGIN) / 3.0, picker.height() - PICKER_MARGIN - 20.0);
        let picked = click(&context, picked, strip, egui::PointerButton::Primary);
        let picked = picked.into_iter().flatten().next().expect("a press on the strip picks");
        assert!(picked[1] >= 250 && picked[0] <= 5, "{picked:?}");
        assert!(picker_frame(&context).is_some());
        // A press outside closes it.
        click(&context, picked, Pos2::new(590.0, 490.0), egui::PointerButton::Primary);
        frame(&context, picked, None, Vec::new());
        let shown = context.memory(|memory| memory.areas().visible_last_frame(&egui::LayerId::new(egui::Order::Foreground, Id::new("colour-test").with("picker"))));
        assert!(!shown, "a press outside closes the picker");
    }

    #[test]
    fn a_small_button_opens_the_picker_at_its_least_size() {
        let context = themed_context();
        let input = |events: Vec<egui::Event>| egui::RawInput { screen_rect: Some(SCREEN), events, ..Default::default() };
        let small = Rect::from_min_size(Pos2::new(40.0, 40.0), Vec2::new(18.0, 30.0));
        let run = |events: Vec<egui::Event>| {
            let output = context.run_ui(input(events), |ui| {
                colour_button(ui, Id::new("small"), small, [0, 0, 255], "", None);
            });
            output.drop_without_applying_deltas();
        };
        let at = small.center();
        let press = |pressed: bool| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
        run(vec![egui::Event::PointerMoved(at)]);
        run(vec![press(true)]);
        run(vec![press(false)]);
        run(Vec::new());
        let picker = context.memory(|memory| memory.area_rect(Id::new("small").with("picker"))).expect("the picker opens");
        assert!((picker.width() - PICKER_MIN_SIZE).abs() < 0.5, "{picker:?}");
    }

    #[test]
    fn a_right_click_types_a_colour_that_the_picker_then_rounds() {
        let context = themed_context();
        let floats = [0.8879231, 0.9046612, 0.9301109];
        let rgb = floats.map(linear_to_srgb);
        let at = BUTTON.center();
        let press = |pressed: bool| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Secondary, pressed, modifiers: egui::Modifiers::NONE };
        for events in [vec![egui::Event::PointerMoved(at)], vec![press(true)], vec![press(false)], Vec::new()] {
            frame(&context, rgb, Some(floats), events);
        }
        let typing = Id::new("colour-test").with("button-typing");
        assert_eq!(slider_popups::typing_text(&context, typing).as_deref(), Some("0.8879, 0.9047, 0.9301"));
        let select_all = egui::Event::Key { key: egui::Key::A, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::COMMAND };
        frame(&context, rgb, Some(floats), vec![select_all]);
        frame(&context, rgb, Some(floats), vec![egui::Event::Text("0.5, 0.5, 0.5".to_string())]);
        let enter = egui::Event::Key { key: egui::Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE };
        let (picked, _) = frame(&context, rgb, Some(floats), vec![enter]);
        // 0.5 as scRGB is byte 188, through the picker 187 or 188.
        let picked = picked.expect("Enter takes the colour");
        assert!(picked.iter().all(|byte| (187..=188).contains(byte)), "{picked:?}");
    }
}
