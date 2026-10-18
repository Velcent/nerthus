//! Markdown as the help windows draw it: the shortcut guide, the change
//! log, update notes and the documentation. CommonMark with tables,
//! strikethrough and GitHub's alerts (parsed by pulldown-cmark): headings,
//! paragraphs, bulleted and numbered lists (nested), quotes and alerts,
//! code blocks, tables and rules, and inside them bold, italics,
//! strikethrough, code and links. Pictures and HTML are left out, but for
//! `<br>`.

use std::iter::Peekable;
use std::ops::Range;

use egui::text::{LayoutJob, TextFormat};
use egui::{Align, Color32, CornerRadius, FontId, Galley, Margin, Pos2, Rect, Sense, Stroke, Ui, UiBuilder, Vec2};
use pulldown_cmark::{Alignment, BlockQuoteKind, Event, HeadingLevel, Parser, Tag, TagEnd};
use std::sync::Arc;

use crate::theme;
use crate::ui::widgets::Tooltips;

/// Body text's size.
const TEXT_SIZE: f32 = 13.0;
/// Space after a paragraph, and after one inside a list item.
const PARAGRAPH_GAP: f32 = 10.0;
const ITEM_PARAGRAPH_GAP: f32 = 2.0;
/// A list's indent, its markers' column and the gap after them.
const LIST_INDENT: f32 = 12.0;
const BULLET_WIDTH: f32 = 10.0;
const NUMBER_WIDTH: f32 = 22.0;
const MARKER_GAP: f32 = 4.0;
/// How far code's plate reaches past its text.
const CODE_PADDING: Vec2 = Vec2::new(3.0, 1.0);

/// A parsed text, drawn as often as it shows.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Document {
    blocks: Vec<Block>,
}

/// A block of the text.
#[derive(Clone, Debug, PartialEq)]
enum Block {
    Heading { level: usize, anchor: String, spans: Vec<Span> },
    Paragraph(Vec<Span>),
    /// Numbered from `start`, else bulleted.
    List { start: Option<u64>, items: Vec<Vec<Block>> },
    Quote { alert: Option<BlockQuoteKind>, blocks: Vec<Block> },
    Code(String),
    Table { alignments: Vec<Alignment>, header: Vec<Vec<Span>>, rows: Vec<Vec<Vec<Span>>> },
    Rule,
}

/// A run of text in one style.
#[derive(Clone, Debug, Default, PartialEq)]
struct Span {
    text: String,
    strong: bool,
    emphasis: bool,
    strikethrough: bool,
    code: bool,
    link: Option<String>,
}

/// What drawing a text did.
#[derive(Debug, Default)]
pub struct Drawn {
    /// The destination of the link clicked.
    pub clicked: Option<String>,
    /// The heading scrolled to.
    pub scrolled: bool,
    /// Link rectangles in the drawing's coordinates. Canvas notes use
    /// these after applying the graph's visual transform.
    pub links: Vec<LinkRegion>,
}

#[derive(Debug)]
pub struct LinkRegion {
    pub rect: Rect,
    pub destination: String,
}

/// How a text is drawn: the heading to scroll to.
#[derive(Clone, Copy, Debug, Default)]
pub struct View<'a> {
    pub scroll_to: Option<&'a str>,
}

/// Parses `text`.
pub fn parse(text: &str) -> Document {
    let parser = Parser::new_ext(text, crate::docs::markdown_options());
    let mut events = parser.peekable();
    let mut anchors = crate::docs::HeadingAnchors::default();
    let blocks = parse_blocks(&mut events, None, &mut anchors);
    Document { blocks }
}

/// Draws `text`, opening the links clicked in the browser.
pub fn show(ui: &mut Ui, text: &str) {
    let document = parse(text);
    let drawn = draw(ui, &document, View::default());
    if let Some(link) = drawn.clicked {
        crate::export_build::open_url(&link);
    }
}

/// Draws `document`, leaving the links clicked to the caller.
pub fn draw(ui: &mut Ui, document: &Document, view: View) -> Drawn {
    draw_document(ui, document, view, false)
}

/// Notes use the same Markdown features as the help, with the 12-point
/// body and heading sizes of Gaea's `MarkdownScrollViewer` style.
pub fn show_note(ui: &mut Ui, text: &str) {
    let document = parse(text);
    let drawn = draw_document(ui, &document, View::default(), true);
    if let Some(link) = drawn.clicked {
        crate::export_build::open_url(&link);
    }
}

/// Paints a canvas note at its graph zoom and returns its links in screen
/// coordinates. The note decides when those links receive pointer input
/// (`PART_NoteText.AllowSelection`), keeping its other space draggable.
pub fn paint_note(ui: &mut Ui, text: &str, rect: Rect, zoom: f32, id: egui::Id) -> Vec<LinkRegion> {
    let document = parse(text);
    let transform = egui::emath::TSTransform { translation: rect.min.to_vec2() * (1.0 - zoom), scaling: zoom };
    let natural_rect = Rect::from_min_size(rect.min, rect.size() / zoom);
    let clip = rect.intersect(ui.clip_rect());
    let natural_clip = transform.inverse().mul_rect(clip);
    let opacity = ui.painter().opacity();
    let mut builder = UiBuilder::new();
    builder = builder.id(id);
    builder = builder.max_rect(natural_rect);
    builder = builder.layout(egui::Layout::top_down(Align::Min));
    builder = builder.disabled();
    let mut note_ui = ui.new_child(builder);
    note_ui.set_opacity(opacity);
    note_ui.set_clip_rect(natural_clip);
    note_ui.spacing_mut().item_spacing.y = 0.0;
    let drawn = note_ui.with_visual_transform(transform, |ui| draw_document(ui, &document, View::default(), true)).inner;
    let mut links = drawn.links;
    for link in &mut links {
        link.rect = transform.mul_rect(link.rect).intersect(clip);
    }
    links.retain(|link| link.rect.is_positive());
    links
}

fn draw_document(ui: &mut Ui, document: &Document, view: View, note: bool) -> Drawn {
    let mut drawn = Drawn::default();
    let mut context = DrawContext { view, drawn: &mut drawn, in_item: false, depth: 0, note };
    draw_blocks(ui, &document.blocks, &mut context);
    drawn
}

type Events<'a> = Peekable<Parser<'a>>;

fn heading_number(level: HeadingLevel) -> usize {
    match level {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    }
}

/// Whether an event belongs inside a paragraph.
fn is_inline(event: &Event) -> bool {
    match event {
        Event::Text(_) | Event::Code(_) | Event::SoftBreak | Event::HardBreak | Event::InlineHtml(_) => true,
        Event::Start(tag) => matches!(tag, Tag::Emphasis | Tag::Strong | Tag::Strikethrough | Tag::Link { .. } | Tag::Image { .. }),
        Event::End(tag) => matches!(tag, TagEnd::Emphasis | TagEnd::Strong | TagEnd::Strikethrough | TagEnd::Link | TagEnd::Image),
        _ => false,
    }
}

/// Blocks until `end` (or the text's end).
fn parse_blocks(events: &mut Events, end: Option<TagEnd>, anchors: &mut crate::docs::HeadingAnchors) -> Vec<Block> {
    let mut blocks = Vec::new();
    while let Some(event) = events.peek() {
        // A tight list's items hold their text without paragraphs.
        if is_inline(event) {
            let spans = parse_inlines(events, None);
            if !spans.is_empty() {
                blocks.push(Block::Paragraph(spans));
            }
            continue;
        }
        let Some(event) = events.next() else {
            break;
        };
        match event {
            Event::End(tag) if Some(tag) == end => return blocks,
            Event::Start(Tag::Paragraph) => {
                let spans = parse_inlines(events, Some(TagEnd::Paragraph));
                blocks.push(Block::Paragraph(spans));
            }
            Event::Start(Tag::Heading { level, .. }) => {
                let spans = parse_inlines(events, Some(TagEnd::Heading(level)));
                let heading = plain(&spans);
                let anchor = anchors.next(&heading);
                blocks.push(Block::Heading { level: heading_number(level), anchor, spans });
            }
            Event::Start(Tag::List(start)) => {
                let mut items = Vec::new();
                while let Some(event) = events.next() {
                    match event {
                        Event::Start(Tag::Item) => items.push(parse_blocks(events, Some(TagEnd::Item), anchors)),
                        Event::End(TagEnd::List(_)) => break,
                        _ => {}
                    }
                }
                blocks.push(Block::List { start, items });
            }
            Event::Start(Tag::BlockQuote(alert)) => {
                let inner = parse_blocks(events, Some(TagEnd::BlockQuote(alert)), anchors);
                blocks.push(Block::Quote { alert, blocks: inner });
            }
            Event::Start(Tag::CodeBlock(_)) => {
                let mut code = String::new();
                for event in events.by_ref() {
                    match event {
                        Event::Text(text) => code.push_str(&text),
                        Event::End(TagEnd::CodeBlock) => break,
                        _ => {}
                    }
                }
                let trimmed = code.strip_suffix('\n').unwrap_or(&code).to_string();
                blocks.push(Block::Code(trimmed));
            }
            Event::Start(Tag::Table(alignments)) => blocks.push(parse_table(events, alignments)),
            Event::Start(Tag::HtmlBlock) => skip_until(events, TagEnd::HtmlBlock),
            Event::Rule => blocks.push(Block::Rule),
            _ => {}
        }
    }
    blocks
}

fn skip_until(events: &mut Events, end: TagEnd) {
    for event in events.by_ref() {
        if event == Event::End(end) {
            break;
        }
    }
}

/// A table's header and rows.
fn parse_table(events: &mut Events, alignments: Vec<Alignment>) -> Block {
    let mut header = Vec::new();
    let mut rows = Vec::new();
    let mut row = Vec::new();
    while let Some(event) = events.next() {
        match event {
            Event::Start(Tag::TableCell) => row.push(parse_inlines(events, Some(TagEnd::TableCell))),
            Event::End(TagEnd::TableHead) => header = std::mem::take(&mut row),
            Event::End(TagEnd::TableRow) => rows.push(std::mem::take(&mut row)),
            Event::End(TagEnd::Table) => break,
            _ => {}
        }
    }
    Block::Table { alignments, header, rows }
}

/// Text until `end`; with none, until the next event that is not part of
/// a paragraph.
fn parse_inlines(events: &mut Events, end: Option<TagEnd>) -> Vec<Span> {
    let mut spans = Vec::new();
    let mut strong = 0;
    let mut emphasis = 0;
    let mut strikethrough = 0;
    let mut links: Vec<String> = Vec::new();
    let mut images = 0;
    loop {
        let open = strong + emphasis + strikethrough + links.len() + images;
        if end.is_none() && open == 0 {
            match events.peek() {
                Some(event) if is_inline(event) => {}
                _ => break,
            }
        }
        let Some(event) = events.next() else {
            break;
        };
        match event {
            Event::End(tag) if Some(tag) == end => break,
            Event::Start(Tag::Strong) => strong += 1,
            Event::End(TagEnd::Strong) => strong -= 1,
            Event::Start(Tag::Emphasis) => emphasis += 1,
            Event::End(TagEnd::Emphasis) => emphasis -= 1,
            Event::Start(Tag::Strikethrough) => strikethrough += 1,
            Event::End(TagEnd::Strikethrough) => strikethrough -= 1,
            Event::Start(Tag::Link { dest_url, .. }) => links.push(dest_url.to_string()),
            Event::End(TagEnd::Link) => {
                links.pop();
            }
            Event::Start(Tag::Image { .. }) => images += 1,
            Event::End(TagEnd::Image) => images -= 1,
            Event::Text(text) if images == 0 => {
                let span = Span {
                    text: text.to_string(),
                    strong: strong > 0,
                    emphasis: emphasis > 0,
                    strikethrough: strikethrough > 0,
                    code: false,
                    link: links.last().cloned(),
                };
                spans.push(span);
            }
            Event::Code(text) if images == 0 => {
                let span = Span { text: text.to_string(), code: true, link: links.last().cloned(), ..Span::default() };
                spans.push(span);
            }
            Event::SoftBreak => spans.push(Span { text: " ".to_string(), ..Span::default() }),
            Event::HardBreak => spans.push(Span { text: "\n".to_string(), ..Span::default() }),
            Event::InlineHtml(html) if is_line_break(&html) => spans.push(Span { text: "\n".to_string(), ..Span::default() }),
            _ => {}
        }
    }
    spans
}

fn is_line_break(html: &str) -> bool {
    let tag: String = html.chars().filter(|character| !character.is_whitespace()).collect::<String>().to_lowercase();
    matches!(tag.as_str(), "<br>" | "<br/>")
}

/// The spans' text, without their styles.
fn plain(spans: &[Span]) -> String {
    let mut text = String::new();
    for span in spans {
        text.push_str(&span.text);
    }
    text
}

/// Where drawing is: the view, what has happened, and whether inside a
/// list item (tighter paragraphs) how many lists deep.
struct DrawContext<'a, 'b> {
    view: View<'a>,
    drawn: &'b mut Drawn,
    in_item: bool,
    depth: usize,
    note: bool,
}

impl DrawContext<'_, '_> {
    fn text_size(&self) -> f32 {
        if self.note { 12.0 } else { TEXT_SIZE }
    }
}

fn draw_blocks(ui: &mut Ui, blocks: &[Block], context: &mut DrawContext) {
    for block in blocks {
        match block {
            Block::Heading { level, anchor, spans } => heading(ui, *level, anchor, spans, context),
            Block::Paragraph(spans) => {
                let style = TextStyle { size: context.text_size(), color: theme::LABEL_DEFAULT, bold: false };
                paragraph(ui, spans, style, context);
                let gap = if context.in_item { ITEM_PARAGRAPH_GAP } else { PARAGRAPH_GAP };
                ui.add_space(gap);
            }
            Block::List { start, items } => list(ui, *start, items, context),
            Block::Quote { alert, blocks } => quote(ui, *alert, blocks, context),
            Block::Code(code) => code_block(ui, code, context),
            Block::Table { alignments, header, rows } => {
                table(ui, alignments, header, rows, context);
                ui.add_space(12.0);
            }
            Block::Rule => {
                ui.add_space(6.0);
                ui.separator();
                ui.add_space(6.0);
            }
        }
    }
}

/// How a run of text is drawn by default.
#[derive(Clone, Copy)]
struct TextStyle {
    size: f32,
    color: Color32,
    bold: bool,
}

fn heading(ui: &mut Ui, level: usize, anchor: &str, spans: &[Span], context: &mut DrawContext) {
    let (size, bold, color, above, below) = match (context.note, level) {
        (true, 1) => (36.0, false, theme::LABEL_EMPHASIS, 24.0, 24.0),
        (true, 2) => (28.0, false, theme::LABEL_EMPHASIS, 24.0, 24.0),
        (true, 3) => (22.0, false, theme::LABEL_DEFAULT, 16.0, 16.0),
        (true, _) => (18.0, false, theme::LABEL_DEFAULT, 6.0, 2.0),
        (false, 1) => (26.0, false, theme::LABEL_EMPHASIS, 12.0, 10.0),
        (false, 2) => (21.0, false, theme::LABEL_EMPHASIS, 14.0, 8.0),
        (false, 3) => (16.0, true, theme::LABEL_EMPHASIS, 10.0, 6.0),
        _ => (14.0, true, theme::ACCENT_PRIMARY, 10.0, 6.0),
    };
    ui.add_space(above);
    let style = TextStyle { size, color, bold };
    let rect = paragraph(ui, spans, style, context);
    ui.add_space(below);
    if !context.drawn.scrolled && context.view.scroll_to == Some(anchor) {
        ui.scroll_to_rect(rect.expand(above), Some(Align::TOP));
        context.drawn.scrolled = true;
    }
}

/// What part of a laid-out text is code, and what part links where.
#[derive(Default)]
struct Marks {
    code: Vec<Range<usize>>,
    links: Vec<(Range<usize>, String)>,
}

/// The spans as one wrapped text, with the character ranges of their code
/// and links. Code has room for its plate's padding on both sides.
fn layout_job(spans: &[Span], style: TextStyle, width: f32) -> (LayoutJob, Marks) {
    let mut job = LayoutJob::default();
    job.wrap.max_width = width;
    let mut marks = Marks::default();
    let mut characters = 0;
    let mut after_code = false;
    for span in spans {
        let font: FontId = if span.code {
            theme::monospace(style.size - 1.0)
        } else if span.strong || style.bold {
            theme::semibold(style.size)
        } else {
            theme::interface(style.size)
        };
        let color = if span.link.is_some() {
            theme::ACCENT_MUTED_GREEN
        } else if span.code {
            theme::ACCENT_PRIMARY
        } else if span.strong {
            theme::LABEL_EMPHASIS
        } else {
            style.color
        };
        let mut format = TextFormat::simple(font, color);
        format.italics = span.emphasis;
        if span.strikethrough {
            format.strikethrough = Stroke::new(1.0, color);
        }
        let count = span.text.chars().count();
        let range = characters..characters + count;
        if span.code {
            marks.code.push(range.clone());
        }
        if let Some(link) = &span.link {
            marks.links.push((range, link.clone()));
        }
        characters += count;
        // The plate's padding before code, and after it before what follows.
        let mut leading_space = 0.0;
        if span.code {
            leading_space += CODE_PADDING.x;
        }
        if after_code {
            leading_space += CODE_PADDING.x;
        }
        job.append(&span.text, leading_space, format);
        after_code = span.code;
    }
    // And after code that ends the text.
    if after_code && let Some(last) = job.sections.last() {
        let format = last.format.clone();
        job.append("", CODE_PADDING.x, format);
    }
    (job, marks)
}

/// Each glyph's character index and rectangle in the galley.
fn glyph_rects(galley: &Galley) -> Vec<(usize, Rect)> {
    let mut rects = Vec::new();
    let mut character = 0;
    for placed in &galley.rows {
        for glyph in &placed.row.glyphs {
            let left = placed.pos.x + glyph.pos.x;
            let rect = Rect::from_min_max(Pos2::new(left, placed.pos.y), Pos2::new(left + glyph.advance_width, placed.pos.y + placed.row.size.y));
            rects.push((character, rect));
            character += 1;
        }
        if placed.ends_with_newline {
            character += 1;
        }
    }
    rects
}

fn link_regions(glyphs: &[(usize, Rect)], marks: &Marks, origin: Pos2) -> Vec<LinkRegion> {
    let mut links: Vec<LinkRegion> = Vec::new();
    for (range, destination) in &marks.links {
        for (character, glyph) in glyphs {
            if !range.contains(character) {
                continue;
            }
            let rect = glyph.translate(origin.to_vec2());
            let previous = links.last_mut();
            if let Some(previous) = previous
                && previous.destination == *destination
                && (previous.rect.top() - rect.top()).abs() < 1.0
            {
                previous.rect = previous.rect.union(rect);
            } else {
                links.push(LinkRegion { rect, destination: destination.clone() });
            }
        }
    }
    links
}

/// The code's plates: per row, the span of its glyphs.
fn code_plates(glyphs: &[(usize, Rect)], code: &[Range<usize>]) -> Vec<Rect> {
    let mut plates: Vec<Rect> = Vec::new();
    let mut current: Option<(usize, Rect)> = None;
    for (range_index, range) in code.iter().enumerate() {
        for (character, rect) in glyphs {
            if !range.contains(character) {
                continue;
            }
            current = match current {
                Some((index, plate)) if index == range_index && (plate.center().y - rect.center().y).abs() < 1.0 => Some((index, plate.union(*rect))),
                Some((_, plate)) => {
                    plates.push(plate);
                    Some((range_index, *rect))
                }
                None => Some((range_index, *rect)),
            };
        }
    }
    if let Some((_, plate)) = current {
        plates.push(plate);
    }
    plates
}

/// A wrapped text at the full width: its code on plates and its links
/// live. Returns where it was drawn.
fn paragraph(ui: &mut Ui, spans: &[Span], style: TextStyle, context: &mut DrawContext) -> Rect {
    let width = ui.available_width().max(40.0);
    let (job, marks) = layout_job(spans, style, width);
    let galley = ui.fonts_mut(|fonts| fonts.layout_job(job));
    let sense = if marks.links.is_empty() { Sense::hover() } else { Sense::click() };
    let (rect, response) = ui.allocate_exact_size(galley.size(), sense);
    paint_text(ui, rect.min, &galley, &marks, style.color);
    let glyphs = glyph_rects(&galley);
    if context.note {
        let links = link_regions(&glyphs, &marks, rect.min);
        context.drawn.links.extend(links);
    }
    if let Some(pointer) = response.hover_pos() {
        let local = pointer - rect.min.to_vec2();
        let hovered = glyphs.iter().find(|(_, glyph)| glyph.contains(local)).map(|(character, _)| *character);
        let link = hovered.and_then(|character| marks.links.iter().find(|(range, _)| range.contains(&character)));
        if let Some((_, link)) = link {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            if link.contains("://") {
                response.clone().tooltip(link.as_str());
            }
            if response.clicked() {
                context.drawn.clicked = Some(link.clone());
            }
        }
    }
    rect
}

/// The code's plates in the galley, padded. The layout leaves the padding
/// clear but where a row wraps, which drops the space before its first
/// glyph: there the plate stays within the text's column.
fn padded_code_plates(galley: &Galley, marks: &Marks) -> Vec<Rect> {
    let glyphs = glyph_rects(galley);
    let right = galley.job.wrap.max_width.max(galley.size().x);
    let mut plates = code_plates(&glyphs, &marks.code);
    for plate in &mut plates {
        *plate = plate.expand2(CODE_PADDING);
        plate.min.x = plate.min.x.max(0.0);
        plate.max.x = plate.max.x.min(right);
    }
    plates
}

/// The galley with its code's plates behind it.
fn paint_text(ui: &Ui, origin: Pos2, galley: &Arc<Galley>, marks: &Marks, color: Color32) {
    if !marks.code.is_empty() {
        for plate in padded_code_plates(galley, marks) {
            let plate = plate.translate(origin.to_vec2());
            ui.painter().rect_filled(plate, 3.0, theme::TEXTBOX_BACKGROUND);
            ui.painter().rect_stroke(plate, 3.0, Stroke::new(1.0, theme::BUTTON_BORDER), egui::StrokeKind::Inside);
        }
    }
    ui.painter().galley(origin, galley.clone(), color);
}

fn list(ui: &mut Ui, start: Option<u64>, items: &[Vec<Block>], context: &mut DrawContext) {
    let marker_width = if start.is_some() { NUMBER_WIDTH } else { BULLET_WIDTH };
    let text_size = context.text_size();
    let line_height = ui.fonts_mut(|fonts| fonts.row_height(&theme::interface(text_size)));
    for (index, item) in items.iter().enumerate() {
        let marker = match start {
            Some(first) => format!("{}.", first + index as u64),
            None if context.depth == 0 => "•".to_string(),
            None => "–".to_string(),
        };
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            ui.add_space(LIST_INDENT);
            let (marker_rect, _) = ui.allocate_exact_size(Vec2::new(marker_width, line_height), Sense::hover());
            let galley = ui.painter().layout_no_wrap(marker, theme::interface(text_size), theme::LABEL_DEFAULT);
            let left = if start.is_some() { marker_rect.right() - galley.size().x } else { marker_rect.left() };
            ui.painter().galley(Pos2::new(left, marker_rect.top()), galley, theme::LABEL_DEFAULT);
            ui.add_space(MARKER_GAP);
            ui.vertical(|ui| {
                let was_in_item = context.in_item;
                context.in_item = true;
                context.depth += 1;
                draw_blocks(ui, item, context);
                context.depth -= 1;
                context.in_item = was_in_item;
            });
        });
        ui.add_space(4.0);
    }
    if !context.in_item {
        ui.add_space(PARAGRAPH_GAP - 4.0);
    }
}

/// A quote, or an alert in its colours with its title.
fn quote(ui: &mut Ui, alert: Option<BlockQuoteKind>, blocks: &[Block], context: &mut DrawContext) {
    let (title, accent, fill) = match alert {
        Some(BlockQuoteKind::Note) => (Some("Note"), theme::ACCENT_STRONG_BLUE, theme::ACCENT_TRANSPARENT_BLUE),
        Some(BlockQuoteKind::Tip) => (Some("Tip"), theme::ACCENT_STRONG_GREEN, theme::ACCENT_TRANSPARENT_GREEN),
        Some(BlockQuoteKind::Important) => (Some("Important"), theme::ACCENT_PURPLE, theme::ACCENT_TRANSPARENT_PURPLE),
        Some(BlockQuoteKind::Warning) => (Some("Warning"), theme::ACCENT_STRONG_ORANGE, theme::ACCENT_TRANSPARENT_ORANGE),
        Some(BlockQuoteKind::Caution) => (Some("Caution"), theme::ACCENT_STRONG_RED, theme::ACCENT_TRANSPARENT_RED),
        None => (None, theme::MENU_SEPARATOR, theme::PANEL_CLADDING),
    };
    let mut frame = egui::Frame::new();
    frame = frame.fill(fill.gamma_multiply(0.45));
    frame = frame.corner_radius(CornerRadius::same(4));
    frame = frame.inner_margin(Margin { left: 14, right: 10, top: 8, bottom: 2 });
    let shown = frame.show(ui, |ui| {
        ui.set_width(ui.available_width());
        if let Some(title) = title {
            let label = egui::RichText::new(title).font(theme::semibold(TEXT_SIZE)).color(theme::LABEL_EMPHASIS);
            ui.label(label);
            ui.add_space(4.0);
        }
        draw_blocks(ui, blocks, context);
    });
    let rect = shown.response.rect;
    let bar = Rect::from_min_max(rect.min, Pos2::new(rect.left() + 3.0, rect.bottom()));
    ui.painter().rect_filled(bar, CornerRadius { nw: 4, sw: 4, ne: 0, se: 0 }, accent);
    ui.add_space(PARAGRAPH_GAP);
}

/// Code on a dark plate, selectable, scrolling sideways when long.
fn code_block(ui: &mut Ui, code: &str, context: &DrawContext) {
    let mut frame = egui::Frame::new();
    frame = frame.fill(theme::TEXTBOX_BACKGROUND);
    frame = frame.stroke(Stroke::new(1.0, theme::BUTTON_BORDER));
    frame = frame.corner_radius(CornerRadius::same(3));
    frame = frame.inner_margin(Margin::same(8));
    frame.show(ui, |ui| {
        ui.set_width(ui.available_width());
        if context.note {
            let font = theme::monospace(context.text_size());
            let job = LayoutJob::simple(code.to_string(), font, theme::ACCENT_MUTED_GREEN, ui.available_width());
            ui.label(job);
        } else {
            let mut scroll = egui::ScrollArea::horizontal();
            scroll = scroll.id_salt(("code-block", code.len(), code.get(..code.len().min(32))));
            scroll.show(ui, |ui| code_field(ui, code));
        }
    });
    ui.add_space(PARAGRAPH_GAP);
}

/// The code selectable, its lines at their full length however narrow the
/// room: a scroll area offers only its own width, and the field would wrap
/// them to it.
fn code_field(ui: &mut Ui, code: &str) -> egui::Response {
    let font = theme::monospace(TEXT_SIZE - 1.0);
    let mut layouter = |ui: &Ui, text: &dyn egui::TextBuffer, _wrap_width: f32| {
        let job = LayoutJob::simple(text.as_str().to_string(), font.clone(), theme::ACCENT_PRIMARY, f32::INFINITY);
        ui.fonts_mut(|fonts| fonts.layout_job(job))
    };
    let width = layouter(ui, &code, f32::INFINITY).size().x;
    ui.set_width(width.max(ui.available_width()));
    let mut text = code;
    let mut field = egui::TextEdit::multiline(&mut text);
    field = field.font(font.clone());
    field = field.text_color(theme::ACCENT_PRIMARY);
    field = field.frame(egui::Frame::NONE);
    field = field.desired_rows(1);
    field = field.desired_width(f32::INFINITY);
    field = field.layouter(&mut layouter);
    ui.add(field)
}

/// The columns' widths: each its text's width when all fit, else at least
/// its longest word's and the rest shared out by what each would like.
fn column_widths(ui: &Ui, header: &[Vec<Span>], rows: &[Vec<Vec<Span>>], width: f32, padding: f32, text_size: f32) -> Vec<f32> {
    let columns = header.len().max(rows.iter().map(Vec::len).max().unwrap_or(0)).max(1);
    let mut natural = vec![0.0f32; columns];
    let mut narrowest = vec![0.0f32; columns];
    let mut measure = |cells: &[Vec<Span>], bold: bool| {
        for (column, cell) in cells.iter().enumerate().take(columns) {
            let style = TextStyle { size: text_size, color: theme::LABEL_DEFAULT, bold };
            let (job, _) = layout_job(cell, style, f32::INFINITY);
            let galley = ui.fonts_mut(|fonts| fonts.layout_job(job));
            natural[column] = natural[column].max(galley.size().x + padding);
            for word in plain(cell).split_whitespace() {
                let word_galley = ui.painter().layout_no_wrap(word.to_string(), theme::interface(text_size), Color32::WHITE);
                narrowest[column] = narrowest[column].max(word_galley.size().x + padding);
            }
        }
    };
    measure(header, true);
    for row in rows {
        measure(row, false);
    }
    let wanted: f32 = natural.iter().sum();
    if wanted <= width {
        // The table spans the width: the room left goes to each column by
        // its share.
        let extra = width - wanted;
        return natural.iter().map(|natural| natural + extra * natural / wanted.max(1.0)).collect();
    }
    let least: f32 = narrowest.iter().sum();
    let room = (width - least).max(0.0);
    let flexible: f32 = natural.iter().zip(&narrowest).map(|(natural, narrowest)| (natural - narrowest).max(0.0)).sum();
    let mut widths = Vec::new();
    for (natural, narrowest) in natural.iter().zip(&narrowest) {
        let share = if flexible > 0.0 { (natural - narrowest).max(0.0) / flexible } else { 1.0 / columns as f32 };
        widths.push(narrowest + room * share);
    }
    widths
}

/// A table: the header in semibold over a line, every other row shaded,
/// cells wrapping within their columns.
fn table(ui: &mut Ui, alignments: &[Alignment], header: &[Vec<Span>], rows: &[Vec<Vec<Span>>], context: &mut DrawContext) {
    let padding = 8.0;
    let width = ui.available_width();
    let text_size = context.text_size();
    let widths = column_widths(ui, header, rows, width, padding * 2.0, text_size);
    let total: f32 = widths.iter().sum();
    let mut draw_row = |ui: &mut Ui, cells: &[Vec<Span>], is_header: bool, shaded: bool| {
        let mut galleys = Vec::new();
        let mut height: f32 = 26.0;
        for (column, cell_width) in widths.iter().enumerate() {
            let spans = cells.get(column).map(Vec::as_slice).unwrap_or_default();
            let style = TextStyle { size: text_size, color: theme::LABEL_DEFAULT, bold: is_header };
            let (job, marks) = layout_job(spans, style, cell_width - padding * 2.0);
            let galley = ui.fonts_mut(|fonts| fonts.layout_job(job));
            height = height.max(galley.size().y + 10.0);
            galleys.push((galley, marks));
        }
        let (rect, response) = ui.allocate_exact_size(Vec2::new(total, height), Sense::click());
        if shaded {
            ui.painter().rect_filled(rect, 0.0, theme::PANEL_CLADDING);
        }
        if is_header {
            let bottom = rect.bottom() - 0.5;
            ui.painter().hline(rect.x_range(), bottom, Stroke::new(1.0, theme::MENU_SEPARATOR));
        }
        let mut left = rect.left();
        for (column, (galley, marks)) in galleys.iter().enumerate() {
            let cell_width = widths[column];
            if context.note {
                let cell_rect = Rect::from_min_size(Pos2::new(left, rect.top()), Vec2::new(cell_width, height));
                ui.painter().rect_stroke(cell_rect, 0.0, Stroke::new(1.0, theme::BUTTON_BORDER_ALT), egui::StrokeKind::Inside);
            }
            let alignment = alignments.get(column).copied().unwrap_or(Alignment::None);
            let text_left = match alignment {
                Alignment::Right => left + cell_width - padding - galley.size().x,
                Alignment::Center => left + (cell_width - galley.size().x) / 2.0,
                _ => left + padding,
            };
            let origin = Pos2::new(text_left, rect.center().y - galley.size().y / 2.0);
            paint_text(ui, origin, galley, marks, theme::LABEL_DEFAULT);
            if context.note {
                let glyphs = glyph_rects(galley);
                let links = link_regions(&glyphs, marks, origin);
                context.drawn.links.extend(links);
            }
            if let Some(pointer) = response.hover_pos() {
                let local = pointer - origin.to_vec2();
                let glyphs = glyph_rects(galley);
                let hovered = glyphs.iter().find(|(_, glyph)| glyph.contains(local)).map(|(character, _)| *character);
                let link = hovered.and_then(|character| marks.links.iter().find(|(range, _)| range.contains(&character)));
                if let Some((_, link)) = link {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                    if response.clicked() {
                        context.drawn.clicked = Some(link.clone());
                    }
                }
            }
            left += cell_width;
        }
    };
    if !header.is_empty() {
        draw_row(ui, header, true, false);
    }
    for (index, row) in rows.iter().enumerate() {
        draw_row(ui, row, false, index % 2 == 1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(text: &str) -> Span {
        Span { text: text.to_string(), ..Span::default() }
    }

    #[test]
    fn inline_styles_are_found() {
        let document = parse("Press `Ctrl + N` or **bold** and *it* [here](https://example.com).");
        let Block::Paragraph(spans) = &document.blocks[0] else {
            panic!("not a paragraph: {document:?}");
        };
        assert_eq!(spans[0], text("Press "));
        assert_eq!(spans[1], Span { text: "Ctrl + N".to_string(), code: true, ..Span::default() });
        assert_eq!(spans[3], Span { text: "bold".to_string(), strong: true, ..Span::default() });
        assert_eq!(spans[5], Span { text: "it".to_string(), emphasis: true, ..Span::default() });
        assert_eq!(spans[7], Span { text: "here".to_string(), link: Some("https://example.com".to_string()), ..Span::default() });
    }

    #[test]
    fn double_backticks_hold_a_backtick() {
        let document = parse("`` ` ``");
        assert_eq!(document.blocks, vec![Block::Paragraph(vec![Span { text: "`".to_string(), code: true, ..Span::default() }])]);
    }

    #[test]
    fn tables_keep_their_alignment() {
        let document = parse("| Command | Shortcut |\n| :--- | ---: |\n| New | `Ctrl + N` |");
        let Block::Table { alignments, header, rows } = &document.blocks[0] else {
            panic!("not a table: {document:?}");
        };
        assert_eq!(alignments, &vec![Alignment::Left, Alignment::Right]);
        assert_eq!(plain(&header[1]), "Shortcut");
        assert_eq!(rows[0][1], vec![Span { text: "Ctrl + N".to_string(), code: true, ..Span::default() }]);
    }

    #[test]
    fn tight_and_nested_lists_hold_their_text() {
        let document = parse("- one\n- two\n  1. inner\n\n***\n");
        let Block::List { start: None, items } = &document.blocks[0] else {
            panic!("not a list: {document:?}");
        };
        assert_eq!(items[0], vec![Block::Paragraph(vec![text("one")])]);
        assert!(matches!(items[1][1], Block::List { start: Some(1), .. }));
        assert_eq!(document.blocks[1], Block::Rule);
    }

    #[test]
    fn alerts_and_headings_are_kept() {
        let document = parse("## Menus and Fast Access\n\n> [!WARNING]\n> Careful.\n");
        let Block::Heading { level: 2, anchor, .. } = &document.blocks[0] else {
            panic!("not a heading: {document:?}");
        };
        assert_eq!(anchor, "menus-and-fast-access");
        assert!(matches!(document.blocks[1], Block::Quote { alert: Some(BlockQuoteKind::Warning), .. }));
    }

    #[test]
    fn heading_anchors_are_unique_across_nested_blocks() {
        let markdown = "# Repeat\n\n> ## Repeat\n\n- ### Repeat\n\n## Repeat-1\n\n## Repeat";
        let document = parse(markdown);
        assert!(matches!(&document.blocks[0], Block::Heading { anchor, .. } if anchor == "repeat"));
        let Block::Quote { blocks, .. } = &document.blocks[1] else {
            panic!("not a quote: {:?}", document.blocks[1]);
        };
        assert!(matches!(&blocks[0], Block::Heading { anchor, .. } if anchor == "repeat-1"));
        let Block::List { items, .. } = &document.blocks[2] else {
            panic!("not a list: {:?}", document.blocks[2]);
        };
        assert!(matches!(&items[0][0], Block::Heading { anchor, .. } if anchor == "repeat-2"));
        assert!(matches!(&document.blocks[3], Block::Heading { anchor, .. } if anchor == "repeat-1-1"));
        assert!(matches!(&document.blocks[4], Block::Heading { anchor, .. } if anchor == "repeat-3"));
    }

    #[test]
    fn repeated_heading_links_scroll_to_each_occurrence() {
        let filler = "Filler paragraph.\n\n".repeat(20);
        let markdown = format!("# Repeated\n\n{filler}## Repeated\n\n{filler}");
        let document = parse(&markdown);
        let mut offsets = Vec::new();

        for anchor in ["repeated", "repeated-1"] {
            with_ui(|ui| {
                let mut scroll = egui::ScrollArea::vertical();
                scroll = scroll.animated(false);
                scroll = scroll.max_height(120.0);
                scroll = scroll.auto_shrink([false, false]);
                let view = View { scroll_to: Some(anchor) };
                let shown = scroll.show(ui, |ui| draw(ui, &document, view));
                assert!(shown.inner.scrolled, "#{anchor} was not found");
                offsets.push(shown.state.offset.y);
            });
        }

        assert!(offsets[0] < 1.0, "the first heading scrolled to {}", offsets[0]);
        assert!(offsets[1] > offsets[0] + 200.0, "the later heading did not scroll farther: {offsets:?}");
    }

    #[test]
    fn breaks_and_pictures() {
        let document = parse("a<br>b ![picture](x.png) c");
        let Block::Paragraph(spans) = &document.blocks[0] else {
            panic!("not a paragraph: {document:?}");
        };
        assert_eq!(plain(spans), "a\nb  c");
    }

    #[test]
    fn canvas_notes_draw_rich_markdown_at_each_zoom_within_the_note() {
        let text = "**Bold** and *italic* [link](https://example.com) with `code`\n\n3. First\n4. Second\n\n| Name | Value |\n| --- | --- |\n| Height | 0.7 |\n\n```text\nterrain = 1\n```";
        let context = egui::Context::default();
        theme::apply(&context);
        for zoom in [0.5, 1.0, 2.0] {
            let rect = Rect::from_min_size(Pos2::new(100.0, 80.0), Vec2::new(300.0, 500.0) * zoom);
            let input = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::splat(1600.0))), ..Default::default() };
            let output = context.run_ui(input, |ui| {
                paint_note(ui, text, rect, zoom, egui::Id::new("markdown-note-test"));
            });
            let text_shapes: Vec<_> = output.shapes.iter().filter_map(|clipped| match &clipped.shape {
                egui::Shape::Text(shape) => Some((clipped.clip_rect, shape)),
                _ => None,
            }).collect();
            let shape_for = |text: &str| {
                text_shapes.iter().find(|(_, shape)| shape.galley.text() == text).map(|(_, shape)| *shape).unwrap_or_else(|| panic!("{text:?} was not drawn at {zoom}"))
            };
            let body = shape_for("Bold and italic link with code");
            let sections = &body.galley.job.sections;
            assert!(sections.iter().any(|section| section.format.italics));
            assert!(sections.iter().any(|section| section.format.font_id.family == theme::semibold(12.0).family));
            assert!(sections.iter().any(|section| section.format.color == theme::ACCENT_MUTED_GREEN));
            assert!(sections.iter().any(|section| section.format.font_id.family == theme::monospace(12.0).family));
            for rendered in ["3.", "4.", "First", "Second", "Name", "Value", "Height", "0.7", "terrain = 1"] {
                shape_for(rendered);
            }
            for (clip, _) in text_shapes {
                assert!((clip.min - rect.min).length() < 0.01 && (clip.max - rect.max).length() < 0.01, "{clip:?} leaves {rect:?}");
            }
            output.drop_without_applying_deltas();
        }
    }

    /// Runs `check` in a frame with the app's fonts.
    fn with_ui(check: impl FnMut(&mut Ui)) {
        let context = egui::Context::default();
        theme::apply(&context);
        let output = context.run_ui(egui::RawInput::default(), check);
        output.drop_without_applying_deltas();
    }

    /// The first paragraph of `markdown` laid out at `width`, with its marks.
    fn laid_out(ui: &Ui, markdown: &str, width: f32) -> (Arc<Galley>, Marks) {
        let document = parse(markdown);
        let Block::Paragraph(spans) = &document.blocks[0] else {
            panic!("not a paragraph: {document:?}");
        };
        let style = TextStyle { size: TEXT_SIZE, color: theme::LABEL_DEFAULT, bold: false };
        let (job, marks) = layout_job(spans, style, width);
        (ui.fonts_mut(|fonts| fonts.layout_job(job)), marks)
    }

    #[test]
    fn code_plates_keep_clear_of_the_text_beside_them() {
        // What's New's and the command-line page's code, with its brackets
        // and the comma between two options.
        let markdown = "`nerthus-build build <project> [options]` saves `.terrain` projects (`report.txt` and `report.json`): `-r`, `--region` or `-p`, `--profile`";
        with_ui(|ui| {
            // On one row, then wrapped onto more and more.
            for width in [2000.0, 300.0, 120.0] {
                let (galley, marks) = laid_out(ui, markdown, width);
                let glyphs = glyph_rects(&galley);
                let characters: Vec<char> = galley.job.text.chars().collect();
                for plate in padded_code_plates(&galley, &marks) {
                    assert!(0.0 <= plate.left() && plate.right() <= width, "{plate:?} leaves the column at {width}");
                    for (character, glyph) in &glyphs {
                        let is_code = marks.code.iter().any(|range| range.contains(character));
                        let same_row = (glyph.center().y - plate.center().y).abs() < 1.0;
                        // Within the rounding of the glyphs to pixels.
                        let overlap = plate.right().min(glyph.right()) - plate.left().max(glyph.left());
                        assert!(is_code || !same_row || overlap <= 1.0, "a plate covers {:?} by {overlap} at {width}", characters[*character]);
                    }
                }
            }
            // The paragraph's first code starts past its plate's padding, so
            // the plate starts at the text's column.
            let (galley, _) = laid_out(ui, markdown, 2000.0);
            assert!(glyph_rects(&galley)[0].1.left() >= CODE_PADDING.x - 0.5);
        });
    }

    #[test]
    fn code_keeps_its_long_lines_whole() {
        let words: Vec<String> = (0..30).map(|column| format!("column{column}")).collect();
        let line = words.join(" ");
        let code = format!("{line}\nshort");
        with_ui(|ui| {
            ui.set_max_width(300.0);
            let font = theme::monospace(TEXT_SIZE - 1.0);
            let line_width = ui.painter().layout_no_wrap(line.clone(), font.clone(), Color32::WHITE).size().x;
            let row_height = ui.fonts_mut(|fonts| fonts.row_height(&font));
            assert!(line_width > 600.0);
            // As wide as the long line, for the scroll area to scroll, and
            // two rows high.
            let response = code_field(ui, &code);
            assert!(response.rect.width() >= line_width, "{} for a line {line_width} wide", response.rect.width());
            assert!(response.rect.height() < 2.5 * row_height, "{} for rows {row_height} high", response.rect.height());
        });
    }

    #[test]
    fn code_draws_box_drawing_a_column_wide() {
        // The build report's rules and separators and the folder trees'
        // branches. `has_glyph` can't tell: Hack, which draws them, also
        // draws the box egui puts in place of what no font has.
        let lines = "─│├└";
        let missing = '\u{378}';
        with_ui(|ui| {
            for font in [theme::monospace(TEXT_SIZE - 1.0), theme::monospace_semibold(TEXT_SIZE), theme::monospace_bold(TEXT_SIZE)] {
                let laid_out = |text: String| ui.painter().layout_no_wrap(text, font.clone(), Color32::WHITE);
                let drawn = |character: char| laid_out(character.to_string()).rows[0].row.glyphs[0].uv_rect;
                let width = |character: char| laid_out(character.to_string().repeat(20)).size().x;
                for line in lines.chars() {
                    assert_ne!(drawn(line), drawn(missing), "{line} draws as a box at {font:?}");
                    // As wide as the letters, for the report's columns to
                    // line up.
                    let difference = width(line) - width('a');
                    assert!(difference.abs() < 1.0, "{line} is off the columns by {difference} in 20 at {font:?}");
                }
            }
        });
    }
}
