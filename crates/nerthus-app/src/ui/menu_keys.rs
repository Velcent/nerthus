//! The menus from the keyboard. The row the keys are on holds egui's
//! focus and is drawn as under the pointer, so Enter and Space click it
//! as they would a focused button. Down and Up (Tab and Shift Tab too)
//! step through the rows that can be chosen, round from the last to the
//! first; Home and End go to the first and the last. Right, Enter or
//! Space on a row with a submenu opens it on its first row; Left or
//! Escape closes it back to its row (Escape in a menu is egui's, which
//! closes it). In the main menu Left, and Right on a row without a
//! submenu, go to the menu beside; F10 (or Alt let go alone) puts the
//! menu bar in menu mode, and Alt with an item's access key opens it
//! (`bar`).
//!
//! While the keys are in use the pointer neither highlights rows nor
//! opens submenus, until it moves or presses.

use egui::containers::menu::{find_menu_root, is_in_menu, MenuState};
use egui::{Context, Event, FocusDirection, Id, InputState, Key, Modifiers, Popup, Response, Ui, UiKind, Vec2};

/// Where the context keeps it.
const KEYBOARD_ID: &str = "nerthus-menu-keys";
/// Alt pressed and let go alone puts the menu bar in menu mode, as in
/// Windows; on macOS Option types characters, so it does not there.
const ALT_ENTERS_MENU_MODE: bool = !cfg!(target_os = "macos");

/// What the menus drawn last showed, and where the keys are.
#[derive(Clone, Default)]
struct Keyboard {
    /// The menus drawn in the last pass, in the order their first rows
    /// were (a submenu after its menu); this pass's as they are drawn.
    menus: Vec<Menu>,
    /// The keys were used last: the pointer has not moved since.
    active: bool,
    /// While the keys are in use, the submenus they opened, each with the
    /// menus it is in: these stay open, and only these.
    path: Vec<Id>,
    /// The menu whose first row takes the keys when it is drawn, and the
    /// pass that asked.
    focus_first: Option<(Id, u64)>,
    /// Left or Right asked for the menu beside, in the menu bar.
    step: Option<Step>,
    /// Menu mode: the menu bar's item highlighted while none is open, and
    /// the keys it took this pass.
    bar_focus: Option<usize>,
    bar_keys: BarKeys,
    /// The menu bar's item open last pass.
    bar_open: Option<usize>,
    /// Alt is held, and nothing else has been pressed since it went down;
    /// it was let go so this pass.
    alt_held: bool,
    alt_alone: bool,
    alt_let_go_alone: bool,
}

/// The keys menu mode takes.
#[derive(Clone, Copy, Default)]
struct BarKeys {
    left: bool,
    right: bool,
    /// Down, Enter or Space: open the item.
    open: bool,
    escape: bool,
    /// A press of the pointer, anywhere, leaves menu mode.
    pressed: bool,
}

#[derive(Clone)]
struct Menu {
    /// The menu's `Ui` (its popup's id).
    id: Id,
    pass: u64,
    /// The rows that can be chosen, in order.
    rows: Vec<Row>,
    /// The row with the focus, and the one under the pointer.
    focused: Option<Id>,
    hovered: Option<Id>,
}

#[derive(Clone, Copy)]
struct Row {
    id: Id,
    /// The submenu it opens.
    submenu: Option<Id>,
}

/// The menu beside the one open, in the menu bar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    Back,
    Forward,
}

/// The menu keys pressed this pass.
#[derive(Clone, Copy, Default)]
struct Keys {
    down: usize,
    up: usize,
    home: bool,
    end: bool,
    left: bool,
    right: bool,
    /// Escape and Enter or Space, left in the input for egui's popups and
    /// the focused row unless a submenu takes them.
    escape: bool,
    enter: bool,
}

impl Keys {
    fn any(&self) -> bool {
        self.down > 0 || self.up > 0 || self.home || self.end || self.left || self.right || self.escape || self.enter
    }
}

/// What a press asks of egui.
#[derive(Default, PartialEq, Debug)]
struct Outcome {
    /// The row to give the focus.
    focus: Option<Id>,
    /// Enter and Space are not a click on the focused row.
    consume_enter: bool,
    /// Escape closes a submenu, not the menus.
    consume_escape: bool,
}

fn with<R>(ctx: &Context, f: impl FnOnce(&mut Keyboard) -> R) -> R {
    ctx.data_mut(|data| f(data.get_temp_mut_or_default::<Keyboard>(Id::new(KEYBOARD_ID))))
}

/// Before anything else in a pass: while a menu is open (and no other
/// widget has the keyboard) its keys are taken from the input, so the
/// shortcuts and egui's own focus moves leave them, and acted on.
pub fn begin(ctx: &Context) {
    let pass = ctx.cumulative_pass_nr();
    let moved = ctx.input(|input| input.pointer.delta() != Vec2::ZERO || input.pointer.any_pressed());
    note_alt(ctx);
    // Menu mode takes its keys before anything else sees them.
    let bar_mode = with(ctx, |keyboard| keyboard.bar_focus.is_some());
    let bar_keys = if bar_mode { ctx.input_mut(take_bar_keys) } else { BarKeys::default() };
    with(ctx, |keyboard| keyboard.bar_keys = bar_keys);
    let focused = ctx.memory(|memory| memory.focused());
    let ours = with(ctx, |keyboard| {
        keyboard.forget(pass, moved);
        let open = !keyboard.menus.is_empty();
        open && focused.is_none_or(|id| keyboard.menu_of(id).is_some())
    });
    if !ours {
        return;
    }
    let keys = ctx.input_mut(take_keys);
    // Tab and the arrows would also move egui's focus off the menu.
    ctx.memory_mut(|memory| memory.move_focus(FocusDirection::None));
    let outcome = with(ctx, |keyboard| keyboard.press(&keys, pass));
    if let Some(id) = outcome.focus {
        ctx.memory_mut(|memory| memory.request_focus(id));
    }
    ctx.input_mut(|input| {
        if outcome.consume_enter {
            input.consume_key(Modifiers::NONE, Key::Enter);
            input.consume_key(Modifiers::NONE, Key::Space);
        }
        if outcome.consume_escape {
            input.consume_key(Modifiers::NONE, Key::Escape);
        }
    });
    if keys.any() {
        ctx.request_repaint();
    }
}

/// Alt going down and up: let go with nothing pressed between, it asks
/// for menu mode.
fn note_alt(ctx: &Context) {
    let (alt, other) = ctx.input(|input| {
        let pressed = input.events.iter().any(|event| matches!(event, Event::Key { pressed: true, .. } | Event::Text(_)));
        (input.modifiers.alt, pressed || input.pointer.any_pressed())
    });
    with(ctx, |keyboard| {
        if alt && !keyboard.alt_held {
            keyboard.alt_alone = true;
        }
        if other {
            keyboard.alt_alone = false;
        }
        keyboard.alt_let_go_alone = ALT_ENTERS_MENU_MODE && !alt && keyboard.alt_held && keyboard.alt_alone;
        keyboard.alt_held = alt;
    });
}

fn take_bar_keys(input: &mut InputState) -> BarKeys {
    let down = input.consume_key(Modifiers::NONE, Key::ArrowDown);
    let enter = input.consume_key(Modifiers::NONE, Key::Enter);
    let space = input.consume_key(Modifiers::NONE, Key::Space);
    BarKeys {
        left: input.consume_key(Modifiers::NONE, Key::ArrowLeft),
        right: input.consume_key(Modifiers::NONE, Key::ArrowRight),
        open: down || enter || space,
        escape: input.consume_key(Modifiers::NONE, Key::Escape),
        pressed: input.pointer.any_pressed(),
    }
}

fn take_keys(input: &mut InputState) -> Keys {
    let back_tab = input.count_and_consume_key(Modifiers::SHIFT, Key::Tab);
    let tab = input.count_and_consume_key(Modifiers::NONE, Key::Tab);
    Keys {
        down: input.count_and_consume_key(Modifiers::NONE, Key::ArrowDown) + tab,
        up: input.count_and_consume_key(Modifiers::NONE, Key::ArrowUp) + back_tab,
        home: input.consume_key(Modifiers::NONE, Key::Home),
        end: input.consume_key(Modifiers::NONE, Key::End),
        left: input.consume_key(Modifiers::NONE, Key::ArrowLeft),
        right: input.consume_key(Modifiers::NONE, Key::ArrowRight),
        escape: input.key_pressed(Key::Escape),
        enter: input.key_pressed(Key::Enter) || input.key_pressed(Key::Space),
    }
}

impl Keyboard {
    /// Keeps the menus the last pass drew; a moved pointer takes over.
    fn forget(&mut self, pass: u64, moved: bool) {
        self.menus.retain(|menu| menu.pass + 1 >= pass);
        self.step = None;
        if moved {
            self.active = false;
        }
        if self.menus.is_empty() {
            self.path.clear();
        }
        if self.focus_first.is_some_and(|(_, asked)| asked + 1 < pass) {
            self.focus_first = None;
        }
    }

    fn menu_of(&self, row: Id) -> Option<usize> {
        self.menus.iter().position(|menu| menu.rows.iter().any(|candidate| candidate.id == row))
    }

    /// The menu `menu` is a submenu of, and its row there.
    fn parent(&self, menu: Id) -> Option<(usize, Id)> {
        self.menus.iter().enumerate().find_map(|(index, candidate)| candidate.rows.iter().find(|row| row.submenu == Some(menu)).map(|row| (index, row.id)))
    }

    /// `menu` and the menus it is in, the deepest first.
    fn chain(&self, menu: Id) -> Vec<Id> {
        let mut chain = vec![menu];
        while let Some((index, _)) = self.parent(*chain.last().unwrap()) {
            let parent = self.menus[index].id;
            if chain.contains(&parent) {
                break;
            }
            chain.push(parent);
        }
        chain
    }

    /// The menu the keys act in and the row they are on: the one under
    /// the pointer once it has moved, else the focused one, else none in
    /// the deepest menu.
    fn current(&self) -> Option<(usize, Option<usize>)> {
        let row_in = |pick: fn(&Menu) -> Option<Id>| {
            self.menus.iter().enumerate().find_map(|(index, menu)| {
                let id = pick(menu)?;
                menu.rows.iter().position(|row| row.id == id).map(|row| (index, Some(row)))
            })
        };
        let hovered = if self.active { None } else { row_in(|menu| menu.hovered) };
        hovered.or_else(|| row_in(|menu| menu.focused)).or_else(|| self.menus.len().checked_sub(1).map(|last| (last, None)))
    }

    fn press(&mut self, keys: &Keys, pass: u64) -> Outcome {
        let mut outcome = Outcome::default();
        if !keys.any() {
            return outcome;
        }
        let Some((menu_index, row_index)) = self.current() else {
            return outcome;
        };
        let menu = &self.menus[menu_index];
        let menu_id = menu.id;
        let row = row_index.map(|index| menu.rows[index]);
        let count = menu.rows.len();
        self.active = true;
        if keys.escape || keys.left {
            match self.parent(menu_id) {
                // Back to the submenu's row.
                Some((parent, row)) => {
                    outcome.focus = Some(row);
                    outcome.consume_escape = keys.escape;
                    self.path = self.chain(self.menus[parent].id);
                }
                None if keys.escape => {}
                None => self.step = Some(Step::Back),
            }
            return outcome;
        }
        let submenu = row.and_then(|row| row.submenu);
        if keys.right || (keys.enter && submenu.is_some()) {
            match submenu {
                Some(child) => {
                    self.open(child, pass, &mut outcome);
                    outcome.consume_enter = true;
                }
                None if keys.right => self.step = Some(Step::Forward),
                None => {}
            }
            if keys.right {
                return outcome;
            }
        }
        if keys.enter {
            match row {
                // The click goes to the row the keys are on.
                Some(row) if submenu.is_none() => outcome.focus = Some(row.id),
                _ => outcome.consume_enter = true,
            }
            return outcome;
        }
        if count == 0 {
            return outcome;
        }
        let mut index = row_index;
        if keys.home {
            index = Some(0);
        }
        if keys.end {
            index = Some(count - 1);
        }
        for _ in 0..keys.down {
            index = Some(index.map_or(0, |index| (index + 1) % count));
        }
        for _ in 0..keys.up {
            index = Some(index.map_or(count - 1, |index| (index + count - 1) % count));
        }
        if let Some(index) = index {
            outcome.focus = Some(self.menus[menu_index].rows[index].id);
            self.path = self.chain(menu_id);
        }
        outcome
    }

    /// Opens `child` from the keys, its first row taking them.
    fn open(&mut self, child: Id, pass: u64, outcome: &mut Outcome) {
        let drawn = self.menus.iter().find(|menu| menu.id == child).and_then(|menu| menu.rows.first());
        match drawn {
            Some(first) => outcome.focus = Some(first.id),
            None => self.focus_first = Some((child, pass)),
        }
        self.path = self.chain(child);
    }

    /// `menu` as drawn this pass.
    fn drawn(&mut self, menu: Id, pass: u64) -> &mut Menu {
        let fresh = Menu { id: menu, pass, rows: Vec::new(), focused: None, hovered: None };
        let index = match self.menus.iter().position(|candidate| candidate.id == menu) {
            Some(index) if self.menus[index].pass == pass => index,
            // Its first draw this pass: it goes after the menus drawn before it.
            Some(index) => {
                self.menus.remove(index);
                self.menus.push(fresh);
                self.menus.len() - 1
            }
            None => {
                self.menus.push(fresh);
                self.menus.len() - 1
            }
        };
        &mut self.menus[index]
    }

    /// Notes a row drawn in `menu` this pass.
    fn note(&mut self, menu: Id, pass: u64, row: Row, focused: bool, hovered: bool) {
        let record = self.drawn(menu, pass);
        record.rows.push(row);
        if focused {
            record.focused = Some(row.id);
        }
        if hovered {
            record.hovered = Some(row.id);
        }
    }
}

/// Notes the menu `ui` is in as open, its rows or not (`menu_style`, and
/// every row, disabled ones too).
pub fn menu(ui: &Ui) {
    if !is_in_menu(ui) {
        return;
    }
    let menu = find_menu_root(ui).id;
    let pass = ui.ctx().cumulative_pass_nr();
    with(ui.ctx(), |keyboard| {
        keyboard.drawn(menu, pass);
    });
}

/// A row that can be chosen in a menu, as drawn (`response`), and the
/// submenu it opens: notes it, gives it the keys if its menu was just
/// opened from them, and says whether it is highlighted (focused while
/// the keys are in use, under the pointer otherwise).
pub fn row(ui: &Ui, response: &Response, submenu: Option<Id>) -> bool {
    if !is_in_menu(ui) {
        return response.hovered();
    }
    let ctx = ui.ctx();
    let menu = find_menu_root(ui).id;
    let pass = ctx.cumulative_pass_nr();
    let first = with(ctx, |keyboard| {
        let first = keyboard.focus_first.is_some_and(|(asked, _)| asked == menu);
        if first {
            keyboard.focus_first = None;
        }
        first
    });
    if first {
        response.request_focus();
    }
    let focused = response.has_focus();
    let hovered = response.hovered();
    let row = Row { id: response.id, submenu };
    let (highlighted, keyed) = with(ctx, |keyboard| {
        keyboard.note(menu, pass, row, focused, hovered);
        if keyboard.active { (focused, true) } else { (hovered, false) }
    });
    // The keys on a row scrolled out of view (`widgets::menu_rows`) bring
    // it back. A popup's `Ui` starts its own stack, so the scroll area is
    // the menu's, not one of what the menu was opened from.
    let scrolls = ui.stack().iter().any(|stack| stack.kind() == Some(UiKind::ScrollArea));
    if keyed && response.gained_focus() && scrolls {
        response.scroll_to_me(None);
    }
    highlighted
}

/// Whether the submenu `child` of a row about to be drawn in `ui` is
/// open. While the keys are in use it is open only when they opened it,
/// and kept open in egui's menu state against the pointer resting
/// elsewhere on its menu.
pub fn submenu_open(ui: &Ui, child: Id) -> bool {
    let ctx = ui.ctx();
    let held = with(ctx, |keyboard| keyboard.active.then(|| keyboard.path.contains(&child)));
    match held {
        Some(true) => {
            // So egui's menu keeps it, as shown.
            MenuState::mark_shown(ctx, child);
            MenuState::from_ui(ui, |state, _| state.open_item = Some(child));
            true
        }
        Some(false) => false,
        None => MenuState::from_ui(ui, |state, _| state.open_item == Some(child)),
    }
}

/// The keys are in use: submenus open only from them.
pub fn active(ctx: &Context) -> bool {
    with(ctx, |keyboard| keyboard.active)
}

/// The menu bar's item highlighted in menu mode, for the bar to draw as
/// under the pointer.
pub fn bar_focus(ctx: &Context) -> Option<usize> {
    with(ctx, |keyboard| keyboard.bar_focus)
}

/// The menu bar's part, once its menus (their popups' ids and access
/// keys, in order) are drawn, as WPF's main menu takes keys. F10, or Alt
/// let go alone, puts the bar in menu mode, its first item highlighted but
/// not open, or takes it out of it, closing a menu open; Alt and an item's
/// access key opens that item. In menu mode Left and Right go between the
/// items, Down, Enter or Space open the one highlighted, and Escape or a
/// press leaves; Escape out of an open menu comes back to its item. Left
/// and Right in an open menu open the one beside. A modal window keeps
/// these keys and cancels menu mode, including a menu already open.
pub fn bar(ctx: &Context, menus: &[(Id, Key)], modal: bool) {
    if modal {
        for (popup, _) in menus {
            Popup::close_id(ctx, *popup);
        }
        with(ctx, |keyboard| *keyboard = Keyboard::default());
        return;
    }
    if menus.is_empty() {
        return;
    }
    let count = menus.len();
    let open = menus.iter().position(|(id, _)| Popup::is_id_open(ctx, *id));
    let step = with(ctx, |keyboard| keyboard.step.take());
    let f10 = ctx.input_mut(|input| input.consume_key(Modifiers::NONE, Key::F10));
    let alt_alone = with(ctx, |keyboard| std::mem::take(&mut keyboard.alt_let_go_alone));
    let toggled = f10 || alt_alone;
    // Alt and a key with no Ctrl, a system key: an access key.
    let bar_mode = with(ctx, |keyboard| keyboard.bar_focus.is_some());
    let access = ctx.input_mut(|input| {
        menus.iter().position(|(_, key)| {
            input.consume_key(Modifiers::ALT, *key) || (bar_mode && input.consume_key(Modifiers::NONE, *key))
        })
    });
    let escape = ctx.input(|input| input.key_pressed(Key::Escape));
    let (focus, keys, was_open) = with(ctx, |keyboard| (keyboard.bar_focus, keyboard.bar_keys, keyboard.bar_open));

    let mut target = None;
    let mut next_focus = focus;
    if let Some(index) = access {
        target = Some(index);
    } else if let Some(open_index) = open {
        match step {
            _ if toggled => {
                Popup::close_id(ctx, menus[open_index].0);
                next_focus = None;
            }
            Some(Step::Back) => target = Some((open_index + count - 1) % count),
            Some(Step::Forward) => target = Some((open_index + 1) % count),
            None => {}
        }
    } else if toggled {
        next_focus = if focus.is_some() { None } else { Some(0) };
    } else if let Some(closed) = was_open.filter(|_| escape) {
        // Escape out of a menu: back to its item.
        next_focus = Some(closed);
    } else if let Some(index) = focus {
        if keys.pressed || keys.escape {
            next_focus = None;
        } else if keys.open {
            target = Some(index);
        } else if keys.left {
            next_focus = Some((index + count - 1) % count);
        } else if keys.right {
            next_focus = Some((index + 1) % count);
        }
    } else if was_open.is_some() {
        next_focus = None;
    }
    if focus != next_focus || toggled {
        ctx.request_repaint();
    }
    let still_open = target.or(open.filter(|_| !toggled));
    with(ctx, |keyboard| {
        keyboard.bar_focus = if still_open.is_some() { None } else { next_focus };
        keyboard.bar_open = still_open;
    });
    let Some(target) = target else {
        return;
    };
    let popup = menus[target].0;
    Popup::open_id(ctx, popup);
    // The keys leave a text field for the menu.
    ctx.memory_mut(|memory| {
        if let Some(focused) = memory.focused() {
            memory.surrender_focus(focused);
        }
    });
    let pass = ctx.cumulative_pass_nr();
    with(ctx, |keyboard| {
        keyboard.active = true;
        keyboard.path.clear();
        keyboard.focus_first = Some((popup, pass));
    });
    ctx.request_repaint();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(name: &str) -> Id {
        Id::new(name)
    }

    fn menu(name: &str, rows: &[(&str, Option<&str>)]) -> Menu {
        let rows = rows.iter().map(|(row, submenu)| Row { id: id(row), submenu: submenu.map(id) }).collect();
        Menu { id: id(name), pass: 1, rows, focused: None, hovered: None }
    }

    /// File with Open Recent's submenu open beside it.
    fn keyboard() -> Keyboard {
        let file = menu("file", &[("new", None), ("open", None), ("recent", Some("recent-menu")), ("exit", None)]);
        let recent = menu("recent-menu", &[("autosave", None)]);
        Keyboard { menus: vec![file, recent], ..Default::default() }
    }

    fn down() -> Keys {
        Keys { down: 1, ..Default::default() }
    }

    #[test]
    fn down_from_nothing_starts_at_the_deepest_menus_first_row_and_wraps() {
        let mut keyboard = keyboard();
        keyboard.menus.pop();
        assert_eq!(keyboard.press(&down(), 2).focus, Some(id("new")));
        assert!(keyboard.active);
        keyboard.menus[0].focused = Some(id("exit"));
        assert_eq!(keyboard.press(&down(), 2).focus, Some(id("new")));
        let up = Keys { up: 1, ..Default::default() };
        assert_eq!(keyboard.press(&up, 2).focus, Some(id("recent")));
        let end = Keys { end: true, ..Default::default() };
        assert_eq!(keyboard.press(&end, 2).focus, Some(id("exit")));
    }

    #[test]
    fn the_row_under_the_pointer_leads_until_the_keys_are_used() {
        let mut keyboard = keyboard();
        keyboard.menus.pop();
        keyboard.menus[0].focused = Some(id("new"));
        keyboard.menus[0].hovered = Some(id("recent"));
        assert_eq!(keyboard.press(&down(), 2).focus, Some(id("exit")));
        // Now the keys lead: the focused row, whatever the pointer rests on.
        keyboard.menus[0].focused = Some(id("exit"));
        assert_eq!(keyboard.press(&down(), 2).focus, Some(id("new")));
    }

    #[test]
    fn right_opens_a_submenu_on_its_first_row_and_left_comes_back() {
        let mut keyboard = keyboard();
        keyboard.menus.pop();
        keyboard.menus[0].focused = Some(id("recent"));
        let right = Keys { right: true, ..Default::default() };
        let opened = keyboard.press(&right, 2);
        // Not drawn yet: its first row takes the keys once it is.
        assert_eq!(opened.focus, None);
        assert_eq!(keyboard.focus_first, Some((id("recent-menu"), 2)));
        assert_eq!(keyboard.path, vec![id("recent-menu"), id("file")]);

        let mut keyboard = self::keyboard();
        keyboard.menus[1].focused = Some(id("autosave"));
        let left = Keys { left: true, ..Default::default() };
        let back = keyboard.press(&left, 2);
        assert_eq!(back.focus, Some(id("recent")));
        assert_eq!(keyboard.path, vec![id("file")]);
        assert_eq!(keyboard.step, None);
    }

    #[test]
    fn enter_on_a_submenu_row_opens_it_instead_of_clicking() {
        let mut keyboard = keyboard();
        keyboard.menus[0].focused = Some(id("recent"));
        keyboard.menus.pop();
        let enter = Keys { enter: true, ..Default::default() };
        let outcome = keyboard.press(&enter, 2);
        assert!(outcome.consume_enter);
        assert_eq!(keyboard.focus_first, Some((id("recent-menu"), 2)));
        // On another row, Enter is left for the row to take as a click.
        keyboard.menus[0].focused = Some(id("open"));
        let outcome = keyboard.press(&enter, 2);
        assert_eq!(outcome, Outcome { focus: Some(id("open")), ..Default::default() });
    }

    #[test]
    fn at_the_top_left_and_right_step_to_the_menus_beside_and_escape_is_egui_s() {
        let mut keyboard = keyboard();
        keyboard.menus.pop();
        keyboard.menus[0].focused = Some(id("open"));
        let left = Keys { left: true, ..Default::default() };
        keyboard.press(&left, 2);
        assert_eq!(keyboard.step, Some(Step::Back));
        let right = Keys { right: true, ..Default::default() };
        keyboard.press(&right, 2);
        assert_eq!(keyboard.step, Some(Step::Forward));
        // Escape is left for egui, which closes the menu.
        let escape = Keys { escape: true, ..Default::default() };
        assert_eq!(keyboard.press(&escape, 2), Outcome::default());
        // In a submenu Escape only closes that.
        let mut keyboard = self::keyboard();
        keyboard.menus[1].focused = Some(id("autosave"));
        let outcome = keyboard.press(&escape, 2);
        assert!(outcome.consume_escape);
        assert_eq!(outcome.focus, Some(id("recent")));
    }
}
