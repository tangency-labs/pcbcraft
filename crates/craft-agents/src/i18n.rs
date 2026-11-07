//! The chat's messages in English. SolidCraft looks these keys up in its translations; PCBCraft
//! has one language for now, so the keys stay and the copied modules keep calling the same
//! functions. Unknown keys show themselves, so a missing message is visible, never a panic.

/// The message for `id`.
pub fn tr(id: &str) -> &str {
    match id {
        "ui.agent_chat_cancel_timeout" => "The agent did not respond to Stop and was disconnected. Reconnect to continue.",
        "ui.agent_chat_cannot_start" => "Installed, but it does not start: {error}",
        "ui.agent_chat_exited" => "The agent stopped.",
        "ui.agent_chat_models_not_a_list" => "The answer was no model list",
        "ui.agent_chat_node_too_old" => "Node.js {version} is too old for this agent. Install Node.js 20.10 or newer, then press Refresh.",
        "ui.agent_chat_reinstall" => {
            "{agent} is installed ({program}), but it did not start when PCBCraft checked it: {error}\nIts installation may be incomplete, or made for another kind of computer. Install it again with `{install}`, check that it prints its version in a terminal, then press Refresh."
        }
        "ui.agent_chat_state_auth" => "Sign-in needed",
        "ui.ai_bad_url" => "Enter a URL that starts with http:// or https://",
        "ui.ai_http_busy" => "the endpoint is busy or over its limit; try again later",
        "ui.ai_http_key" => "the endpoint refused the key: check it in Preferences > AI",
        "ui.ai_http_not_found" => "no such endpoint or model: check the base URL and the model name",
        "ui.ai_http_too_large" => "the conversation is too large for this endpoint; start a new chat",
        "ui.ai_key_needs_https" => "A key is only sent over HTTPS, or to this computer.",
        "ui.ai_mode_all" => "Allow everything",
        "ui.ai_mode_ask" => "Ask every time",
        "ui.ai_mode_edits" => "Auto-allow project edits",
        "ui.ai_need_model" => "Enter the model name",
        "ui.ai_no_answer" => "The agent did not answer in time",
        "ui.ai_request_timed_out" => {
            "The model took longer than {seconds} s. You can raise the request timeout for this endpoint in Preferences › AI."
        }
        "ui.ai_test_acp_ok" => "Answers: {agent}",
        "ui.ai_test_answered" => "Reached: {model} answered",
        "ui.ai_test_no_model" => "Reached: {count} models, but not {model}",
        "ui.ai_test_ok" => "Reached: {count} models",
        other => other,
    }
}

/// The message for `id` with each `{name}` replaced by its value. Values are inserted once, so a
/// value holding `{other}` stays as written.
pub fn format(id: &str, args: &[(&str, &str)]) -> String {
    let mut out = String::new();
    let mut rest = tr(id);
    while let Some(start) = rest.find('{') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        match after.find('}').map(|end| (&after[..end], &after[end + 1..])) {
            Some((name, tail)) if let Some((_, value)) = args.iter().find(|(key, _)| *key == name) => {
                out.push_str(value);
                rest = tail;
            }
            _ => {
                out.push('{');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Placeholders are filled once; unknown ones and keys stay visible.
    #[test]
    fn messages_fill_their_placeholders_once() {
        assert_eq!(format("ui.ai_test_ok", &[("count", "3")]), "Reached: 3 models");
        assert_eq!(format("ui.ai_test_answered", &[("model", "{count}")]), "Reached: {count} answered");
        assert_eq!(format("ui.ai_test_no_model", &[("count", "2")]), "Reached: 2 models, but not {model}");
        assert_eq!(tr("ui.unknown"), "ui.unknown");
    }
}
