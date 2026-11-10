/// Keep modified X available to canvas tools while preserving native text cut and Alt keys.
pub(crate) fn canvas_shift_x(key: &winit::keyboard::Key, modifiers: egui::Modifiers, keyboard_focus: bool) -> bool {
    !keyboard_focus
        && modifiers.command
        && modifiers.shift
        && !modifiers.alt
        && matches!(key, winit::keyboard::Key::Character(text) if text.eq_ignore_ascii_case("x"))
}

#[cfg(test)]
mod shortcut_tests {
    use super::canvas_shift_x;
    use egui::Modifiers;
    use winit::keyboard::Key;

    /// Windows supplies Ctrl and Command together; macOS supplies mac_cmd and Command.
    #[test]
    fn shift_x_reaches_canvas_but_not_text_widgets() {
        let windows = Modifiers { ctrl: true, command: true, shift: true, ..Modifiers::NONE };
        let mac = Modifiers { mac_cmd: true, command: true, shift: true, ..Modifiers::NONE };
        for modifiers in [windows, mac] {
            assert!(canvas_shift_x(&Key::Character("X".into()), modifiers, false));
            assert!(canvas_shift_x(&Key::Character("x".into()), modifiers, false));
            assert!(!canvas_shift_x(&Key::Character("X".into()), modifiers, true));
            assert!(!canvas_shift_x(&Key::Character("X".into()), Modifiers { shift: false, ..modifiers }, false));
            assert!(!canvas_shift_x(&Key::Character("X".into()), Modifiers { alt: true, ..modifiers }, false));
            assert!(!canvas_shift_x(&Key::Character("C".into()), modifiers, false));
        }
    }
}
