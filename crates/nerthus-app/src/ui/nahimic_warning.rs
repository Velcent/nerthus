//! The Nahimic warning (`NahimicBlocker.ShowNahimicWarningMessage`): what
//! the software may do to Nerthus, with "Do not warn me again" and Close.
//! It shows by itself as Nerthus starts when Nahimic's service is running,
//! and again from "Nahimic Detected!" on the Start dialog or the status
//! bar.

use egui::RichText;

use crate::theme;
use crate::ui::prompt::{task_button, TaskDialog};

const HEADING: &str = "Nahimic may prevent Nerthus from working";
const TEXT: &str = "Nerthus has detected the 'Nahimic' service. It is an audio augmentation utility for games that comes pre-installed on many computers. Unfortunately, this is a very aggressive application that may try to disrupt Nerthus.\n\nIt may try to intercept Nerthus and in turn may break rendering, or make it impossible to use the Nerthus UI.\n\nWe recommend excluding Nerthus from it, or uninstalling the service completely. As it is an external application that forces itself onto Nerthus, we have no control over it.";

/// The warning while it shows.
#[derive(Default)]
pub struct NahimicWarning {
    /// "Do not warn me again" (`VerificationText`), unticked at first.
    do_not_warn: bool,
}

/// Draws the warning. Returns whether it stays open; once it closes with
/// the box ticked, it no longer shows by itself.
pub fn show(ctx: &egui::Context, warning: &mut NahimicWarning) -> bool {
    let mut closed = false;
    let modal = TaskDialog::new(ctx, egui::Id::new("nahimic-warning"), 460.0);
    let response = modal.show(ctx, |ui, task| {
        task.body(ui, HEADING, true, |ui| {
            ui.add(egui::Label::new(TEXT).wrap());
        });
        let verification = |ui: &mut egui::Ui| {
            let verification = RichText::new("Do not warn me again").font(theme::interface(12.0)).color(theme::LABEL_DEFAULT);
            ui.checkbox(&mut warning.do_not_warn, verification);
        };
        task.buttons_with_verification(ui, verification, |ui| {
            if task_button(ui, "Close").clicked() {
                closed = true;
            }
        });
    });
    if !closed && !response.should_close() {
        return true;
    }
    if warning.do_not_warn {
        crate::nahimic::stop_warning();
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::widgets::fit_checks;

    #[test]
    fn the_warning_keeps_its_instruction_close_and_verification_in_small_windows() {
        let context = fit_checks::context();
        let mut warning = NahimicWarning::default();
        for screen in fit_checks::SMALL_SCREENS {
            let words = fit_checks::settle(&context, screen, |ctx| {
                assert!(show(ctx, &mut warning));
            });
            let expected = [HEADING, "Close", "Do not warn me again"];
            fit_checks::assert_fits(&context, screen, egui::Id::new("nahimic-warning"), &words, &expected);
        }
    }
}
