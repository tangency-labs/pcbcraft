//! #324: every reporting entry point opens one local report draft instead of a blank GitHub form.

use super::*;
use egui::accesskit::Role;

/// Browser tabs the last frame asked to open.
fn opened(h: &Harness<'_, PcbApp>) -> Vec<String> {
    h.output()
        .platform_output
        .commands
        .iter()
        .filter_map(|command| match command {
            egui::OutputCommand::OpenUrl(url) => Some(url.url.clone()),
            _ => None,
        })
        .collect()
}

/// Text the last frame put on the clipboard.
fn copied(h: &Harness<'_, PcbApp>) -> Vec<String> {
    h.output()
        .platform_output
        .commands
        .iter()
        .filter_map(|command| match command {
            egui::OutputCommand::CopyText(text) => Some(text.clone()),
            _ => None,
        })
        .collect()
}

/// Click `label` and collect the browser tabs and clipboard text of the next few frames.
fn click_and_collect(h: &mut Harness<'_, PcbApp>, label: &str) -> (Vec<String>, Vec<String>) {
    h.get_by_label(label).scroll_to_me();
    h.run();
    h.get_by_label(label).click();
    let (mut urls, mut texts) = (Vec::new(), Vec::new());
    for _ in 0..4 {
        h.step();
        urls.extend(opened(h));
        texts.extend(copied(h));
    }
    h.run();
    (urls, texts)
}

/// The draft's editable technical details, which only the report form has.
fn details(h: &Harness<'_, PcbApp>) -> String {
    h.get_by_role_and_label(Role::MultilineTextInput, "Technical details").value().unwrap_or_default()
}

/// Help → Report an Issue, the app bar's Report a bug and About's report action open the same
/// local draft with reviewable technical details. None of them opens a browser tab by itself.
#[test]
fn every_report_entry_point_opens_the_local_draft() {
    let mut h = harness();
    click(&mut h, "Help");
    assert_eq!(click_and_collect(&mut h, "Report an Issue…").0, Vec::<String>::new());
    assert!(details(&h).contains(&format!("PCBCraft {}", display_version())), "{}", details(&h));
    assert!(details(&h).contains("Screen: Schematic"), "{}", details(&h));
    click(&mut h, "Close");
    assert!(h.query_by_label("Continue on GitHub").is_none());

    new_project(&mut h);
    // Status messages can name files, projects, nets or parts; none of them reaches the details.
    h.state_mut().message = Some("Could not open /home/demo-user/demo-secret-board: permission denied".into());
    assert_eq!(click_and_collect(&mut h, "Report a bug").0, Vec::<String>::new());
    assert!(details(&h).contains("Project: open"), "{}", details(&h));
    assert!(!details(&h).contains("demo-secret-board") && !details(&h).contains("/home/"), "{}", details(&h));
    click(&mut h, "Close");

    choose(&mut h, "Help", "About PCBCraft");
    assert_eq!(click_and_collect(&mut h, "Report an Issue…").0, Vec::<String>::new());
    h.get_by_label("Continue on GitHub");
}

/// Copy works for an incomplete draft. Continue on GitHub waits for a description, then opens
/// the prefilled form with the reviewed answers. Neither changes the open project or its revision.
#[test]
fn the_draft_copies_and_hands_off_without_touching_the_project() {
    let mut h = harness();
    new_project(&mut h);
    let revision = h.state().session().revision();
    let encode = |h: &Harness<'_, PcbApp>| pcbcraft_engine::persistence::encode_documents(h.state().session().snapshot().unwrap()).unwrap();
    let saved = encode(&h);
    choose(&mut h, "Help", "Report an Issue…");

    let (urls, text) = click_and_collect(&mut h, "Copy report");
    assert!(urls.is_empty(), "{urls:?}");
    assert_eq!(text.len(), 1, "{text:?}");
    assert!(text[0].starts_with("Bug report\n\n## What happened\n"), "{}", text[0]);
    assert!(text[0].contains(&format!("PCBCraft {}", display_version())), "{}", text[0]);
    assert!(h.get_by_label("Continue on GitHub").accesskit_node().is_disabled());

    h.get_by_role_and_label(Role::MultilineTextInput, "What happened").focus();
    h.run();
    h.get_by_role_and_label(Role::MultilineTextInput, "What happened").type_text("Ω wire & label");
    h.run();
    let (urls, text) = click_and_collect(&mut h, "Continue on GitHub");
    assert_eq!(urls.len(), 1, "{urls:?}");
    assert!(urls[0].starts_with("https://github.com/tangency-labs/pcbcraft/issues/new?title=%CE%A9%20wire%20%26%20label&body="), "{}", urls[0]);
    assert!(urls[0].contains("%23%23%20What%20happened%0A%0A%CE%A9%20wire%20%26%20label"), "{}", urls[0]);
    assert!(text.is_empty(), "a report that fits the link is not copied again");
    h.get_by_label("GitHub opened in your browser with this report filled in. No issue is created until you submit the form there.");

    click(&mut h, "Close");
    assert_eq!(h.state().session().revision(), revision);
    assert_eq!(encode(&h), saved);
}
