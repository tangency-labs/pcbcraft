//! The local issue report every reporting entry point opens. The draft stays in the app until
//! the user copies it or continues to a prefilled GitHub issue form; nothing is submitted for them.

use egui::Ui;
use pcbcraft_chrome::{controls, shell};

/// The repository's new-issue form.
pub(crate) const ISSUE_URL: &str = "https://github.com/tangency-labs/pcbcraft/issues/new";
/// GitHub sends a signed-out visitor through this sign-in page, encoding the issue URL again.
const SIGN_IN_PREFIX: &str = "https://github.com/login?return_to=";
/// Longest sign-in redirect, in bytes, the handoff link may produce. Longer reports fall back
/// to a title-only link and the copied report.
const MAX_SIGN_IN_URL: usize = 8000;
/// Character limit for each answer and for the technical details.
const MAX_FIELD: usize = 4000;
/// Character limit for the summary, which becomes the issue title.
const MAX_SUMMARY: usize = 200;

/// One report being written. Every field is the user's to edit, including the generated
/// technical details.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Draft {
    pub summary: String,
    pub description: String,
    pub steps: String,
    pub expected: String,
    pub details: String,
    /// What the last Copy or Continue did, shown under the actions.
    pub notice: Option<&'static str>,
}

/// What the dialog wants after this frame.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Outcome {
    Open,
    Closed,
}

impl Draft {
    /// Start a draft with generated technical details the user can review and edit.
    pub fn new(details: String) -> Self {
        Self { details: limited(&details, MAX_FIELD), ..Self::default() }
    }

    /// The issue title: the summary, else the description's first line, else a plain label.
    pub fn title(&self) -> String {
        let summary = self.summary.trim();
        if !summary.is_empty() {
            return limited(summary, MAX_SUMMARY);
        }
        match self.description.lines().map(str::trim).find(|line| !line.is_empty()) {
            Some(line) => limited(line, 80),
            None => "Bug report".to_owned(),
        }
    }

    /// The issue body in Markdown. Empty optional answers are left out; the description
    /// section always appears so an incomplete copied draft still shows what is missing.
    pub fn body(&self) -> String {
        let mut body = format!("## What happened\n\n{}\n", self.description.trim());
        for (heading, text) in [("Steps to reproduce", &self.steps), ("Expected result", &self.expected)] {
            if !text.trim().is_empty() {
                body.push_str(&format!("\n## {heading}\n\n{}\n", text.trim()));
            }
        }
        if !self.details.trim().is_empty() {
            body.push_str(&format!("\n## Technical details\n\n```text\n{}\n```\n", self.details.trim().replace("```", "'''")));
        }
        body
    }

    /// The full report for the clipboard: the title line, then the body.
    pub fn text(&self) -> String {
        format!("{}\n\n{}", self.title(), self.body())
    }

    /// GitHub needs a description; copying works for any draft.
    pub fn can_continue(&self) -> bool {
        !self.description.trim().is_empty()
    }
}

/// Keep at most `limit` characters, cutting on a character boundary.
fn limited(text: &str, limit: usize) -> String {
    text.chars().take(limit).collect()
}

/// Percent-encode a query value: unreserved ASCII stays, every other UTF-8 byte becomes `%XX`.
fn encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            out.push(char::from(byte));
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

/// The prefilled new-issue link and whether it carries the whole body. When the sign-in
/// redirect for the full link would pass [`MAX_SIGN_IN_URL`], the link carries the title only.
pub(crate) fn issue_url(title: &str, body: &str) -> (String, bool) {
    let full = format!("{ISSUE_URL}?title={}&body={}", encode(title), encode(body));
    if SIGN_IN_PREFIX.len() + encode(&full).len() <= MAX_SIGN_IN_URL { (full, true) } else { (format!("{ISSUE_URL}?title={}", encode(title)), false) }
}

const COPIED: &str = "Report copied. Paste it wherever you need it.";
const OPENED: &str = "GitHub opened in your browser. Nothing is sent until you submit the form there.";
const TOO_LONG: &str = "The report is too long for a link, so it was copied. Paste it into the GitHub form that opened.";

/// Show the report form in a modal. Copy works for any draft; Continue on GitHub needs a
/// description and opens the prefilled form, copying the report too when it does not fit the link.
/// Neither action touches the open project.
pub(crate) fn show(ctx: &egui::Context, draft: &mut Draft) -> Outcome {
    let mut close = false;
    let closing = shell::scrolling_dialog_width(ctx, "issue_report", "Report an Issue", 520.0, |ui| {
        form(ui, draft);
        ui.horizontal_wrapped(|ui| {
            if controls::button(ui, "Copy report").clicked() {
                ui.ctx().copy_text(draft.text());
                draft.notice = Some(COPIED);
            }
            if controls::enabled_button(ui, draft.can_continue(), "Continue on GitHub").clicked() {
                let (url, complete) = issue_url(&draft.title(), &draft.body());
                if !complete {
                    ui.ctx().copy_text(draft.text());
                }
                ui.ctx().open_url(egui::OpenUrl::new_tab(url));
                draft.notice = Some(if complete { OPENED } else { TOO_LONG });
            }
            close = controls::button(ui, "Close").clicked();
        });
        if let Some(notice) = draft.notice {
            ui.label(shell::dim(notice));
        } else if !draft.can_continue() {
            ui.label(shell::dim("Describe what happened to continue on GitHub."));
        }
    });
    if closing || close { Outcome::Closed } else { Outcome::Open }
}

/// The labelled answers and the editable technical details.
fn form(ui: &mut Ui, draft: &mut Draft) {
    let label = ui.label("Summary (optional)");
    let summary = controls::text(ui, &mut draft.summary).labelled_by(label.id);
    if summary.changed() {
        draft.summary = limited(&draft.summary, MAX_SUMMARY);
    }
    for (label, id, value, rows) in [
        ("What happened", "issue_report_description", &mut draft.description, 4),
        ("Steps to reproduce (optional)", "issue_report_steps", &mut draft.steps, 3),
        ("Expected result (optional)", "issue_report_expected", &mut draft.expected, 2),
    ] {
        ui.label(label);
        controls::text_area(ui, label, egui::Id::unique(id), value, rows, MAX_FIELD);
    }
    ui.label("Technical details");
    ui.label(shell::dim("Review these before sharing and remove anything private."));
    controls::text_area(ui, "Technical details", egui::Id::unique("issue_report_details"), &mut draft.details, 5, MAX_FIELD);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Reserved, space, newline and non-ASCII characters are encoded byte by byte.
    #[test]
    fn query_values_encode_reserved_and_unicode_text() {
        assert_eq!(encode("a b&c=d?#+/\n"), "a%20b%26c%3Dd%3F%23%2B%2F%0A");
        assert_eq!(encode("Ω µ"), "%CE%A9%20%C2%B5");
        assert_eq!(encode("Az09-._~"), "Az09-._~");
    }

    /// The title falls back from the summary to the first description line to a label, and
    /// empty optional answers are left out of the body.
    #[test]
    fn title_and_body_follow_the_answers() {
        let mut draft = Draft::new("PCBCraft 0.1.0".into());
        assert_eq!(draft.title(), "Bug report");
        assert!(!draft.can_continue());
        draft.description = "\n  Wire ends jump  \nafter undo".into();
        assert_eq!(draft.title(), "Wire ends jump");
        assert!(draft.can_continue());
        draft.summary = "Undo moves wire ends".into();
        assert_eq!(draft.title(), "Undo moves wire ends");
        let body = draft.body();
        assert!(body.starts_with("## What happened\n\nWire ends jump  \nafter undo\n"), "{body}");
        assert!(!body.contains("Steps to reproduce") && !body.contains("Expected result"));
        assert!(body.ends_with("## Technical details\n\n```text\nPCBCraft 0.1.0\n```\n"), "{body}");
        draft.steps = "1. Draw a wire".into();
        assert!(draft.body().contains("## Steps to reproduce\n\n1. Draw a wire\n"));
        assert_eq!(draft.text(), format!("Undo moves wire ends\n\n{}", draft.body()));
    }

    /// A short report goes whole into the link; a long one falls back to the title only, and
    /// the cut-over counts the sign-in redirect's second encoding.
    #[test]
    fn long_reports_fall_back_to_a_title_only_link() {
        let (url, complete) = issue_url("Crash & burn", "Line one\nline two");
        assert!(complete);
        assert_eq!(url, format!("{ISSUE_URL}?title=Crash%20%26%20burn&body=Line%20one%0Aline%20two"));
        let long = "é".repeat(2000);
        let (url, complete) = issue_url("Crash", &long);
        assert!(!complete);
        assert_eq!(url, format!("{ISSUE_URL}?title=Crash"));
        let fits = |n: usize| issue_url("t", &"x".repeat(n)).1;
        let edge = (0..MAX_SIGN_IN_URL).take_while(|&n| fits(n)).count();
        let (url, _) = issue_url("t", &"x".repeat(edge - 1));
        assert!(SIGN_IN_PREFIX.len() + encode(&url).len() <= MAX_SIGN_IN_URL);
        assert!(!fits(edge));
    }

    /// The details keep their characters but cannot close the fenced block early.
    #[test]
    fn details_stay_inside_their_code_block() {
        let draft = Draft { description: "x".into(), details: "a ``` b".into(), ..Draft::default() };
        assert!(draft.body().contains("```text\na ''' b\n```"));
    }
}
