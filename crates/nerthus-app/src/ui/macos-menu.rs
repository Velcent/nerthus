//! AppKit menus use the same command tree as the in-window menu bar.
//!
//! AppKit invokes targets between editor frames. The target only queues a
//! command; the editor drains that queue on its next main-thread UI pass.

use std::cell::RefCell;
use std::rc::Rc;

use objc2::rc::Retained;
use objc2::runtime::Sel;
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{NSApplication, NSEventModifierFlags, NSMenu, NSMenuItem};
use objc2_foundation::{NSObject, NSObjectProtocol, NSString};

use super::commands::Command;
use super::main_menu::{MainMenu, MenuEntry};
use super::tool_commands::ToolCommand;

#[derive(Default)]
struct MenuTargetIvars {
    commands: RefCell<Vec<Command>>,
    pending: Rc<RefCell<Vec<Command>>>,
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements, and this class has
    // no Drop implementation. AppKit only uses its target on the main thread.
    #[unsafe(super = NSObject)]
    #[thread_kind = MainThreadOnly]
    #[ivars = MenuTargetIvars]
    #[name = "NerthusMenuTarget"]
    struct MenuTarget;

    // SAFETY: NSObjectProtocol has no additional requirements.
    unsafe impl NSObjectProtocol for MenuTarget {}

    impl MenuTarget {
        // SAFETY: The selector receives its NSMenuItem sender and returns void.
        #[unsafe(method(performMenuCommand:))]
        fn perform_menu_command(&self, sender: &NSMenuItem) {
            let Ok(index) = usize::try_from(sender.tag()) else {
                return;
            };
            let commands = self.ivars().commands.borrow();
            let Some(command) = commands.get(index) else {
                return;
            };
            self.ivars().pending.borrow_mut().push(command.clone());
        }
    }
);

impl MenuTarget {
    fn new(main_thread: MainThreadMarker, pending: Rc<RefCell<Vec<Command>>>) -> Retained<Self> {
        let ivars = MenuTargetIvars { commands: RefCell::default(), pending };
        let target = Self::alloc(main_thread).set_ivars(ivars);
        // SAFETY: This initializes the NSObject superclass with its standard
        // init method before any menu can refer to the target.
        unsafe { msg_send![super(target), init] }
    }

    fn add_command(&self, command: Command) -> isize {
        let mut commands = self.ivars().commands.borrow_mut();
        let tag = commands.len() as isize;
        commands.push(command);
        tag
    }
}

struct NativeMenu {
    menus: Vec<MainMenu>,
    enabled: bool,
    text_input: bool,
    node_keys: bool,
    pending: Rc<RefCell<Vec<Command>>>,
    // NSMenuItem targets are not retained by AppKit. Keep this alive for as
    // long as its menu is installed on NSApplication.
    _target: Retained<MenuTarget>,
    _menu: Retained<NSMenu>,
}

struct MenuBuilder<'a> {
    target: &'a MenuTarget,
    enabled: bool,
    text_input: bool,
    node_keys: bool,
    main_thread: MainThreadMarker,
}

thread_local! {
    static NATIVE_MENU: RefCell<Option<NativeMenu>> = const { RefCell::new(None) };
}

/// Install changed menus and return selections from the previous UI frame.
/// The caller runs on the main thread so AppKit and winit share their menu.
pub(super) fn sync(menus: &[MainMenu], enabled: bool, text_input: bool, node_keys: bool) -> Vec<Command> {
    let Some(main_thread) = MainThreadMarker::new() else {
        return Vec::new();
    };

    NATIVE_MENU.with(|native_menu| {
        let mut stored_menu = native_menu.borrow_mut();
        let pending = match stored_menu.as_ref() {
            Some(menu) => Rc::clone(&menu.pending),
            None => Rc::default(),
        };
        let mut commands = if enabled {
            std::mem::take(&mut *pending.borrow_mut())
        } else {
            pending.borrow_mut().clear();
            Vec::new()
        };
        if text_input {
            commands.retain(|command| !editing_command(command));
        }

        let unchanged = stored_menu.as_ref().is_some_and(|menu| {
            menu.menus == menus && menu.enabled == enabled && menu.text_input == text_input && menu.node_keys == node_keys
        });
        if unchanged {
            return commands;
        }

        let target = MenuTarget::new(main_thread, Rc::clone(&pending));
        let builder = MenuBuilder { target: &target, enabled, text_input, node_keys, main_thread };
        let main_menu = NSMenu::new(main_thread);
        main_menu.setAutoenablesItems(false);

        let app = NSApplication::sharedApplication(main_thread);
        let services_menu = create_menu("Services", main_thread);
        let application_menu = builder.build_application_menu(&app, &services_menu);
        append_submenu(&main_menu, "Nerthus", &application_menu, main_thread);

        let mut help_menu = None;
        for menu in menus {
            let native_submenu = builder.build_menu(menu);
            append_submenu(&main_menu, &menu.title, &native_submenu, main_thread);
            if menu.title == "Help" {
                help_menu = Some(native_submenu);
            }
        }

        app.setMainMenu(Some(&main_menu));
        app.setServicesMenu(Some(&services_menu));
        app.setHelpMenu(help_menu.as_deref());

        let installed_menu =
            NativeMenu { menus: menus.to_vec(), enabled, text_input, node_keys, pending, _target: target, _menu: main_menu };
        *stored_menu = Some(installed_menu);
        commands
    })
}

/// Crash reporting cannot build an editor menu, but must still stop queued
/// and existing editor actions from changing the document behind the report.
pub(super) fn disable() {
    if MainThreadMarker::new().is_none() {
        return;
    }
    NATIVE_MENU.with(|native_menu| {
        let mut stored_menu = native_menu.borrow_mut();
        let Some(menu) = stored_menu.as_mut() else {
            return;
        };
        menu.pending.borrow_mut().clear();
        if menu.enabled {
            disable_commands(&menu._menu);
            menu.enabled = false;
        }
    });
}

fn disable_commands(menu: &NSMenu) {
    let items = menu.itemArray();
    for index in 0..items.count() {
        let item = items.objectAtIndex(index);
        if item.action() == Some(sel!(performMenuCommand:)) {
            item.setEnabled(false);
        }
        if let Some(submenu) = item.submenu() {
            disable_commands(&submenu);
        }
    }
}

fn create_menu(title: &str, main_thread: MainThreadMarker) -> Retained<NSMenu> {
    let title = NSString::from_str(title);
    let menu = NSMenu::initWithTitle(NSMenu::alloc(main_thread), &title);
    menu.setAutoenablesItems(false);
    menu
}

fn append_submenu(parent: &NSMenu, title: &str, submenu: &NSMenu, main_thread: MainThreadMarker) {
    let item = create_item(title, None, "", main_thread);
    item.setSubmenu(Some(submenu));
    parent.addItem(&item);
}

fn create_item(title: &str, action: Option<Sel>, key: &str, main_thread: MainThreadMarker) -> Retained<NSMenuItem> {
    let title = NSString::from_str(title);
    let key = NSString::from_str(key);
    // SAFETY: Callers pass either no action, our correctly typed target
    // selector, or a documented NSApplication action selector.
    unsafe { NSMenuItem::initWithTitle_action_keyEquivalent(NSMenuItem::alloc(main_thread), &title, action, &key) }
}

impl MenuBuilder<'_> {
    fn append_command(&self, menu: &NSMenu, label: &str, shortcut: &str, command: &Command, enabled: bool) -> Retained<NSMenuItem> {
        let allow_text_shortcut = !self.text_input || application_command(command);
        let allow_shortcut = allow_text_shortcut && (self.node_keys || !super::node_key(command));
        let shortcut = if matches!(command, Command::Redo) { "Ctrl Shift Z" } else { shortcut };
        let (key, modifiers) = key_equivalent(shortcut, allow_shortcut);
        let item = create_item(label, Some(sel!(performMenuCommand:)), &key, self.main_thread);
        item.setKeyEquivalentModifierMask(modifiers);
        let enabled = self.enabled && enabled && (!self.text_input || !editing_command(command));
        item.setEnabled(enabled);
        item.setTag(self.target.add_command(command.clone()));
        // SAFETY: The target implements performMenuCommand: and is retained by
        // NativeMenu until this entire menu has been replaced on NSApplication.
        unsafe { item.setTarget(Some(self.target)) };
        menu.addItem(&item);
        item
    }

    fn build_menu(&self, menu: &MainMenu) -> Retained<NSMenu> {
        let native_menu = create_menu(&menu.title, self.main_thread);
        let mut separator_pending = false;
        let mut has_item = false;

        for entry in &menu.entries {
            if matches!(entry, MenuEntry::Separator) {
                separator_pending = has_item;
                continue;
            }
            if let MenuEntry::Action { command: Some(command), .. } = entry {
                if application_command(command) {
                    continue;
                }
            }
            if separator_pending {
                native_menu.addItem(&NSMenuItem::separatorItem(self.main_thread));
                separator_pending = false;
            }
            match entry {
                MenuEntry::Action { label, shortcut, enabled: item_enabled, checked, command, tooltip, .. } => {
                    let item = match command {
                        Some(command) => self.append_command(&native_menu, label, shortcut, command, *item_enabled),
                        None => {
                            let item = create_item(label, None, "", self.main_thread);
                            item.setEnabled(self.enabled && *item_enabled);
                            native_menu.addItem(&item);
                            item
                        }
                    };
                    if let Some(checked) = checked {
                        item.setState(isize::from(*checked));
                    }
                    let tooltip = tooltip.as_ref().map(|text| NSString::from_str(text));
                    item.setToolTip(tooltip.as_deref());
                }
                MenuEntry::Submenu(submenu) => {
                    let native_submenu = self.build_menu(submenu);
                    append_submenu(&native_menu, &submenu.title, &native_submenu, self.main_thread);
                }
                MenuEntry::Separator => unreachable!(),
            }
            has_item = true;
        }
        native_menu
    }

    fn build_application_menu(&self, app: &NSApplication, services_menu: &NSMenu) -> Retained<NSMenu> {
        let menu = create_menu("Nerthus", self.main_thread);
        let about = Command::Tools(ToolCommand::ShowAbout);
        self.append_command(&menu, "About Nerthus", "", &about, true);
        menu.addItem(&NSMenuItem::separatorItem(self.main_thread));
        self.append_command(&menu, "Settings…", "Ctrl ,", &Command::ShowOptions(None), true);
        menu.addItem(&NSMenuItem::separatorItem(self.main_thread));
        append_submenu(&menu, "Services", services_menu, self.main_thread);
        menu.addItem(&NSMenuItem::separatorItem(self.main_thread));
        self.append_application_action(&menu, "Hide Nerthus", "Ctrl H", sel!(hide:), app);
        self.append_application_action(&menu, "Hide Others", "Ctrl Alt H", sel!(hideOtherApplications:), app);
        self.append_application_action(&menu, "Show All", "", sel!(unhideAllApplications:), app);
        menu.addItem(&NSMenuItem::separatorItem(self.main_thread));
        self.append_command(&menu, "Quit Nerthus", "Ctrl Q", &Command::Exit, true);
        menu
    }

    fn append_application_action(&self, menu: &NSMenu, label: &str, shortcut: &str, action: Sel, app: &NSApplication) {
        let (key, modifiers) = key_equivalent(shortcut, true);
        let item = create_item(label, Some(action), &key, self.main_thread);
        item.setKeyEquivalentModifierMask(modifiers);
        // SAFETY: These selectors are NSApplication's documented menu actions.
        // The shared NSApplication remains alive for the process lifetime.
        unsafe { item.setTarget(Some(app)) };
        menu.addItem(&item);
    }
}

fn application_command(command: &Command) -> bool {
    matches!(command, Command::Exit | Command::ShowOptions(None) | Command::Tools(ToolCommand::ShowAbout))
}

fn editing_command(command: &Command) -> bool {
    super::node_key(command) || matches!(command, Command::Undo | Command::Redo | Command::CopyProperties | Command::PasteProperties)
}

/// Unmodified letters must remain ordinary key events. Editor equivalents
/// are suppressed while editing text so egui receives clipboard and undo
/// keyboard events directly; application actions remain available.
fn key_equivalent(shortcut: &str, enabled: bool) -> (String, NSEventModifierFlags) {
    let empty = (String::new(), NSEventModifierFlags::empty());
    if !enabled {
        return empty;
    }
    let mut modifiers = NSEventModifierFlags::empty();
    let mut key = None;
    for token in shortcut.split_whitespace() {
        match token {
            "Ctrl" | "Cmd" => modifiers |= NSEventModifierFlags::Command,
            "Alt" => modifiers |= NSEventModifierFlags::Option,
            "Shift" => modifiers |= NSEventModifierFlags::Shift,
            _ if key.is_none() => key = Some(token),
            _ => return empty,
        }
    }
    let Some(key) = key else {
        return empty;
    };
    if key.chars().count() == 1 {
        if modifiers.is_empty() {
            return empty;
        }
        return (key.to_lowercase(), modifiers);
    }
    if let Some(number) = key.strip_prefix('F').and_then(|value| value.parse::<u32>().ok()) {
        if (1..=35).contains(&number) {
            let function_key = char::from_u32(0xf704 + number - 1).expect("valid AppKit function key");
            return (function_key.to_string(), modifiers);
        }
    }
    empty
}

#[cfg(test)]
mod tests {
    use super::key_equivalent;
    use objc2_app_kit::NSEventModifierFlags;

    #[test]
    fn command_shortcuts_use_macos_modifiers() {
        let command_shift = NSEventModifierFlags::Command | NSEventModifierFlags::Shift;
        assert_eq!(key_equivalent("Ctrl Shift S", true), ("s".into(), command_shift));

        let command_option = NSEventModifierFlags::Command | NSEventModifierFlags::Option;
        assert_eq!(key_equivalent("Ctrl Alt O", true), ("o".into(), command_option));
        assert_eq!(key_equivalent("Cmd ,", true), (",".into(), NSEventModifierFlags::Command));
    }

    #[test]
    fn function_keys_use_appkit_characters() {
        assert_eq!(key_equivalent("F2", true), ("\u{f705}".into(), NSEventModifierFlags::empty()));
        assert_eq!(key_equivalent("Ctrl F6", true), ("\u{f709}".into(), NSEventModifierFlags::Command));
        assert_eq!(key_equivalent("F12", true), ("\u{f70f}".into(), NSEventModifierFlags::empty()));
    }

    #[test]
    fn ordinary_typing_and_blocked_shortcuts_have_no_equivalent() {
        let empty = (String::new(), NSEventModifierFlags::empty());
        for shortcut in ["", "Q", "Del", "Ctrl PageUp", "Ctrl F99", "Ctrl Q extra"] {
            assert_eq!(key_equivalent(shortcut, true), empty);
        }
        assert_eq!(key_equivalent("Ctrl C", false), empty);
        assert_eq!(key_equivalent("F2", false), empty);
    }
}
