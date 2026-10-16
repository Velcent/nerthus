//! Paint SVG paths using Cairo-compatible Rust coverage masks.
//! Resvg still handles SVG filters, patterns and image elements.

#![allow(non_snake_case)]

use resvg::tiny_skia::{self, Mask, Paint, Pixmap, PixmapPaint, Rect, Transform};
use resvg::usvg::{self, Node};

use super::raster;

pub(super) fn render(tree: &usvg::Tree, transform: Transform, pixmap: &mut Pixmap) {
    renderGroup(tree.root(), transform, pixmap);
}

fn renderGroup(group: &usvg::Group, parent_transform: Transform, pixmap: &mut Pixmap) {
    let transform = parent_transform.pre_concat(group.transform());
    if group.opacity().get() == 1.0 {
        for node in group.children() {
            renderNode(node, transform, pixmap);
        }
    } else if let Some(mut layer) = Pixmap::new(pixmap.width(), pixmap.height()) {
        for node in group.children() {
            renderNode(node, transform, &mut layer);
        }
        let paint = PixmapPaint { opacity: group.opacity().get(), ..PixmapPaint::default() };
        pixmap.draw_pixmap(0, 0, layer.as_ref(), &paint, Transform::identity(), None);
    }
}

fn renderNode(node: &Node, transform: Transform, pixmap: &mut Pixmap) {
    match node {
        Node::Group(group) if group.clip_path().is_none()
            && group.mask().is_none()
            && group.filters().is_empty()
            && group.blend_mode() == usvg::BlendMode::Normal => {
                renderGroup(group, transform, pixmap);
            }
        Node::Path(path) if supportsPaint(path) => renderPath(path, transform, pixmap),
        _ => {
            if let Some(bounds) = node.abs_layer_bounding_box() {
                // render_node positions its bounding box at the origin;
                // preserve the document's coordinates when using it here.
                let positioned = transform.pre_translate(bounds.x(), bounds.y());
                resvg::render_node(node, positioned, &mut pixmap.as_mut());
            }
        }
    }
}

fn supportsPaint(path: &usvg::Path) -> bool {
    let pattern_fill = path.fill().is_some_and(|fill| matches!(fill.paint(), usvg::Paint::Pattern(_)));
    let pattern_stroke = path.stroke().is_some_and(|stroke| matches!(stroke.paint(), usvg::Paint::Pattern(_)));
    !pattern_fill && !pattern_stroke
}

fn renderPath(path: &usvg::Path, transform: Transform, pixmap: &mut Pixmap) {
    if !path.is_visible() {
        return;
    }
    if path.paint_order() == usvg::PaintOrder::FillAndStroke {
        renderFill(path, transform, pixmap);
        renderStroke(path, transform, pixmap);
    } else {
        renderStroke(path, transform, pixmap);
        renderFill(path, transform, pixmap);
    }
}

fn renderFill(path: &usvg::Path, transform: Transform, pixmap: &mut Pixmap) {
    let Some(fill) = path.fill() else { return };
    let even_odd = fill.rule() == usvg::FillRule::EvenOdd;
    let Some(mut paint) = convertPaint(fill.paint(), fill.opacity(), transform) else { return };
    if !path.rendering_mode().use_shape_antialiasing() {
        paint.anti_alias = false;
        let rule = if even_odd { tiny_skia::FillRule::EvenOdd } else { tiny_skia::FillRule::Winding };
        // The shader is already in device coordinates.
        if let Some(device_path) = path.data().clone().transform(transform) {
            pixmap.fill_path(&device_path, &paint, rule, Transform::identity(), None);
        }
        return;
    }
    if let Some(mask) = raster::coverage(path.data(), transform, pixmap.width(), pixmap.height(), even_odd) {
        paintMask(pixmap, &paint, fill.paint(), fill.opacity(), &mask);
    } else if let Some(device_path) = path.data().clone().transform(transform) {
        let rule = if even_odd { tiny_skia::FillRule::EvenOdd } else { tiny_skia::FillRule::Winding };
        pixmap.fill_path(&device_path, &paint, rule, Transform::identity(), None);
    }
}

fn renderStroke(path: &usvg::Path, transform: Transform, pixmap: &mut Pixmap) {
    let Some(stroke) = path.stroke() else { return };
    let Some(mut paint) = convertPaint(stroke.paint(), stroke.opacity(), transform) else { return };
    let stroke_style = stroke;
    let stroke = stroke.to_tiny_skia();
    let (scale_x, scale_y) = transform.get_scale();
    let resolution = scale_x.max(scale_y);
    let dashed;
    let mut geometry = path.data();
    if let Some(dash) = &stroke.dash {
        let Some(path) = geometry.dash(dash, resolution) else { return };
        dashed = path;
        geometry = &dashed;
    }
    let Some(outline) = geometry.stroke(&stroke, resolution) else { return };
    if !path.rendering_mode().use_shape_antialiasing() {
        paint.anti_alias = false;
        if let Some(device_path) = outline.transform(transform) {
            pixmap.fill_path(&device_path, &paint, tiny_skia::FillRule::Winding, Transform::identity(), None);
        }
        return;
    }
    if let Some(mask) = raster::coverage(&outline, transform, pixmap.width(), pixmap.height(), false) {
        paintMask(pixmap, &paint, stroke_style.paint(), stroke_style.opacity(), &mask);
    } else if let Some(device_path) = outline.transform(transform) {
        pixmap.fill_path(&device_path, &paint, tiny_skia::FillRule::Winding, Transform::identity(), None);
    }
}

fn paintMask(pixmap: &mut Pixmap, paint: &Paint<'_>, source: &usvg::Paint, opacity: usvg::Opacity, mask: &Mask) {
    if let usvg::Paint::Color(color) = source {
        // Cairo stores solid source colours at 16-bit precision. Pixman
        // takes their high bytes, then uses rounded /255 multiplication
        // for the mask and OVER operation (tiny-skia's low precision
        // pipeline instead uses (product+255)/256).
        let opacity = opacity.get() as f64;
        let components = [color.red, color.green, color.blue, 255];
        let mut premultiplied = [0u8; 4];
        for (target, component) in premultiplied.iter_mut().zip(components) {
            let short = (component as f64 * 257.0 * opacity).round_ties_even() as u32;
            *target = (short >> 8) as u8;
        }
        for (pixel, &coverage) in pixmap.data_mut().chunks_exact_mut(4).zip(mask.data()) {
            if coverage == 0 {
                continue;
            }
            let alpha = multiply(premultiplied[3], coverage);
            for (destination, &source) in pixel.iter_mut().zip(&premultiplied) {
                let foreground = multiply(source, coverage);
                let background = multiply(*destination, 255 - alpha);
                *destination = foreground.saturating_add(background);
            }
        }
        return;
    }
    let Some(bounds) = Rect::from_xywh(0.0, 0.0, pixmap.width() as f32, pixmap.height() as f32) else { return };
    pixmap.fill_rect(bounds, paint, Transform::identity(), Some(mask));
}

fn multiply(value: u8, alpha: u8) -> u8 {
    let product = value as u32 * alpha as u32 + 128;
    ((product + (product >> 8)) >> 8) as u8
}

fn convertPaint(paint: &usvg::Paint, opacity: usvg::Opacity, transform: Transform) -> Option<Paint<'static>> {
    let mut result = Paint::default();
    result.shader = match paint {
        usvg::Paint::Color(color) => {
            result.set_color_rgba8(color.red, color.green, color.blue, opacity.to_u8());
            return Some(result);
        }
        usvg::Paint::LinearGradient(gradient) => {
            let (spread, stops) = gradientStops(gradient, opacity);
            let start = (gradient.x1(), gradient.y1()).into();
            let end = (gradient.x2(), gradient.y2()).into();
            let gradient_transform = transform.pre_concat(gradient.transform());
            tiny_skia::LinearGradient::new(start, end, stops, spread, gradient_transform)?
        }
        usvg::Paint::RadialGradient(gradient) => {
            let (spread, stops) = gradientStops(gradient, opacity);
            let start = (gradient.fx(), gradient.fy()).into();
            let end = (gradient.cx(), gradient.cy()).into();
            let gradient_transform = transform.pre_concat(gradient.transform());
            tiny_skia::RadialGradient::new(start, end, gradient.r().get(), stops, spread, gradient_transform)?
        }
        usvg::Paint::Pattern(_) => return None,
    };
    Some(result)
}

fn gradientStops(gradient: &usvg::BaseGradient, opacity: usvg::Opacity) -> (tiny_skia::SpreadMode, Vec<tiny_skia::GradientStop>) {
    let spread = match gradient.spread_method() {
        usvg::SpreadMethod::Pad => tiny_skia::SpreadMode::Pad,
        usvg::SpreadMethod::Reflect => tiny_skia::SpreadMode::Reflect,
        usvg::SpreadMethod::Repeat => tiny_skia::SpreadMode::Repeat,
    };
    let mut stops = Vec::with_capacity(gradient.stops().len());
    for stop in gradient.stops() {
        let alpha = stop.opacity() * opacity;
        let color = tiny_skia::Color::from_rgba8(stop.color().red, stop.color().green, stop.color().blue, alpha.to_u8());
        stops.push(tiny_skia::GradientStop::new(stop.offset().get(), color));
    }
    (spread, stops)
}
