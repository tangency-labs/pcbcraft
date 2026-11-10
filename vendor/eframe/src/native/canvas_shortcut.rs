/// Keep modified X available to canvas tools while preserving native text cut and Alt keys.
/// Unmapped character keys fall back to physical X, matching egui-winit's non-Latin shortcuts;
/// mapped logical keys retain their layout meaning and named keys keep the native path.
pub(crate) fn canvas_shift_x(
    key: &winit::keyboard::Key,
    physical_key: &winit::keyboard::PhysicalKey,
    modifiers: egui::Modifiers,
    keyboard_focus: bool,
) -> bool {
    !keyboard_focus
        && modifiers.command
        && modifiers.shift
        && !modifiers.alt
        && matches!(key, winit::keyboard::Key::Character(text) if egui::Key::from_name(text)
            .map_or(*physical_key == winit::keyboard::PhysicalKey::Code(winit::keyboard::KeyCode::KeyX), |key| key == egui::Key::X))
}

#[cfg(test)]
mod shortcut_tests {
    use super::canvas_shift_x;
    use egui::Modifiers;
    use winit::keyboard::{Key, KeyCode, NamedKey, PhysicalKey};

    /// Windows supplies Ctrl and Command together; macOS supplies mac_cmd and Command.
    #[test]
    fn shift_x_reaches_canvas_but_not_text_widgets() {
        let windows = Modifiers { ctrl: true, command: true, shift: true, ..Modifiers::NONE };
        let mac = Modifiers { mac_cmd: true, command: true, shift: true, ..Modifiers::NONE };
        let physical_x = PhysicalKey::Code(KeyCode::KeyX);
        for modifiers in [windows, mac] {
            assert!(canvas_shift_x(&Key::Character("X".into()), &physical_x, modifiers, false));
            assert!(canvas_shift_x(&Key::Character("x".into()), &physical_x, modifiers, false));
            assert!(!canvas_shift_x(&Key::Character("X".into()), &physical_x, modifiers, true));
            assert!(!canvas_shift_x(&Key::Character("X".into()), &physical_x, Modifiers { shift: false, ..modifiers }, false));
            assert!(!canvas_shift_x(&Key::Character("X".into()), &physical_x, Modifiers { alt: true, ..modifiers }, false));
            assert!(!canvas_shift_x(&Key::Character("C".into()), &physical_x, modifiers, false));
        }
    }

    /// Physical fallback covers non-Latin characters without overriding mapped letters or Delete.
    #[test]
    fn non_latin_shift_x_uses_physical_fallback_without_changing_cut_keys() {
        let physical_x = PhysicalKey::Code(KeyCode::KeyX);
        let physical_c = PhysicalKey::Code(KeyCode::KeyC);
        for modifiers in [
            Modifiers { ctrl: true, command: true, shift: true, ..Modifiers::NONE },
            Modifiers { mac_cmd: true, command: true, shift: true, ..Modifiers::NONE },
        ] {
            for text in ["ч", "Ч", "χ", "Χ"] {
                let key = Key::Character(text.into());
                assert!(canvas_shift_x(&key, &physical_x, modifiers, false));
                assert!(!canvas_shift_x(&key, &physical_c, modifiers, false));
                assert!(!canvas_shift_x(&key, &physical_x, modifiers, true));
                assert!(!canvas_shift_x(&key, &physical_x, Modifiers { shift: false, ..modifiers }, false));
                assert!(!canvas_shift_x(&key, &physical_x, Modifiers { alt: true, ..modifiers }, false));
            }
            assert!(canvas_shift_x(&Key::Character("x".into()), &physical_c, modifiers, false));
            assert!(!canvas_shift_x(&Key::Character("c".into()), &physical_x, modifiers, false));
            assert!(!canvas_shift_x(&Key::Named(NamedKey::Delete), &PhysicalKey::Code(KeyCode::Delete), modifiers, false));
        }
        assert!(!canvas_shift_x(&Key::Named(NamedKey::Delete), &PhysicalKey::Code(KeyCode::Delete), Modifiers::SHIFT, false));
    }
}
