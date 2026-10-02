//! What every adapter does to the transcript before converting it, after pi's
//! `transformMessages`: images a model cannot see become a note, another model's thinking
//! becomes plain text, tool call ids get the shape the API wants, failed turns are left out,
//! a call that never got a result gets one, and a request carries only its latest images.

use crate::types::{AssistantMessage, AssistantPart, ContentPart, LlmMessage, StopReason, ToolResultMessage};

pub const NON_VISION_USER_IMAGE_PLACEHOLDER: &str = "(image omitted: model does not support images)";
pub const NON_VISION_TOOL_IMAGE_PLACEHOLDER: &str = "(tool image omitted: model does not support images)";
pub const OLDER_IMAGE_PLACEHOLDER: &str = "(older image omitted: a request carries only the latest images)";
pub const NO_RESULT_PROVIDED: &str = "No result provided";

/// The most images one request carries. Anthropic refuses an image over 2000 pixels on a side in
/// a request with more than 20, and every image costs its tokens again on each request.
pub const MAX_REQUEST_IMAGES: usize = 20;

/// The most image data, in base64, one request carries: under the 20 MB a Gemini request takes
/// and the 32 MB an Anthropic one does, with room for the text.
pub const MAX_REQUEST_IMAGE_BYTES: usize = 16 * 1024 * 1024;

/// Past the image count, older images leave ten at a time, so the prompt's cached prefix breaks
/// once in ten new images rather than at each.
const IMAGE_DROP_STEP: usize = 10;

pub struct TransformOptions<'a> {
    /// The provider and model the request goes to. A message from the same pair is "own":
    /// its thinking seals and server blocks are kept, its tool call ids left alone.
    pub provider: &'a str,
    pub model: &'a str,
    pub supports_images: bool,
    /// Rewrites a tool call id from another model into the shape this API accepts.
    pub normalize_tool_call_id: Option<fn(&str) -> String>,
}

pub fn transform_messages(messages: &[LlmMessage], options: &TransformOptions<'_>) -> Vec<LlmMessage> {
    transform_messages_with_origins(messages, options).into_iter().map(|(_, message)| message).collect()
}

/// [`transform_messages`], each message paired with the index in `messages` it comes from. A
/// result made up for a call that never got one belongs to the message it follows.
pub fn transform_messages_with_origins(messages: &[LlmMessage], options: &TransformOptions<'_>) -> Vec<(usize, LlmMessage)> {
    let mut id_map: Vec<(String, String)> = Vec::new();
    let mut transformed: Vec<LlmMessage> = Vec::with_capacity(messages.len());

    for message in messages {
        match message {
            LlmMessage::User(user) => {
                let mut user = user.clone();
                if !options.supports_images {
                    user.content = replace_images(&user.content, NON_VISION_USER_IMAGE_PLACEHOLDER);
                }
                transformed.push(LlmMessage::User(user));
            }
            LlmMessage::ToolResult(result) => {
                let mut result = result.clone();
                if !options.supports_images {
                    result.content = replace_images(&result.content, NON_VISION_TOOL_IMAGE_PLACEHOLDER);
                }
                if let Some((_, normalized)) = id_map.iter().find(|(original, _)| *original == result.tool_call_id) {
                    result.tool_call_id = normalized.clone();
                }
                transformed.push(LlmMessage::ToolResult(result));
            }
            LlmMessage::Assistant(assistant) => {
                let own = assistant.provider == options.provider && assistant.model == options.model;
                let mut out = assistant.clone();
                out.content = Vec::with_capacity(assistant.content.len());
                for part in &assistant.content {
                    match part {
                        AssistantPart::Thinking { thinking, signature } => {
                            if own && signature.as_deref().is_some_and(|s| !s.is_empty()) {
                                out.content.push(part.clone());
                            } else if thinking.trim().is_empty() {
                                // Nothing to say and no seal to keep.
                            } else if own {
                                out.content.push(part.clone());
                            } else {
                                out.content.push(AssistantPart::Text { text: thinking.clone() });
                            }
                        }
                        AssistantPart::Text { .. } => out.content.push(part.clone()),
                        AssistantPart::ToolCall(call) => {
                            let mut call = call.clone();
                            if !own {
                                if let Some(normalize) = options.normalize_tool_call_id {
                                    let normalized = normalize(&call.id);
                                    if normalized != call.id {
                                        id_map.push((call.id.clone(), normalized.clone()));
                                        call.id = normalized;
                                    }
                                }
                            }
                            out.content.push(AssistantPart::ToolCall(call));
                        }
                        AssistantPart::ServerBlock { .. } => {
                            if own {
                                out.content.push(part.clone());
                            }
                        }
                    }
                }
                transformed.push(LlmMessage::Assistant(out));
            }
        }
    }

    // Second pass: a failed or aborted turn is not replayed, and every tool call gets a
    // result before the next assistant turn, the next user message, or the end.
    let mut result: Vec<(usize, LlmMessage)> = Vec::with_capacity(transformed.len());
    let mut pending: Vec<(String, String)> = Vec::new();
    let mut answered: Vec<String> = Vec::new();
    fn settle(result: &mut Vec<(usize, LlmMessage)>, pending: &mut Vec<(String, String)>, answered: &mut Vec<String>) {
        let origin = result.last().map_or(0, |(origin, _)| *origin);
        for (id, name) in pending.drain(..) {
            if !answered.contains(&id) {
                result.push((
                    origin,
                    LlmMessage::ToolResult(ToolResultMessage {
                        tool_call_id: id,
                        tool_name: name,
                        content: vec![ContentPart::text(NO_RESULT_PROVIDED)],
                        details: serde_json::Value::Null,
                        is_error: true,
                        timestamp: crate::now_ms(),
                    }),
                ));
            }
        }
        answered.clear();
    }
    for (origin, message) in transformed.into_iter().enumerate() {
        match message {
            LlmMessage::Assistant(assistant) => {
                settle(&mut result, &mut pending, &mut answered);
                if matches!(assistant.stop_reason, StopReason::Error | StopReason::Aborted) {
                    continue;
                }
                let calls: Vec<(String, String)> = assistant.tool_calls().iter().map(|c| (c.id.clone(), c.name.clone())).collect();
                if !calls.is_empty() {
                    pending = calls;
                }
                result.push((origin, LlmMessage::Assistant(assistant)));
            }
            LlmMessage::ToolResult(tool_result) => {
                answered.push(tool_result.tool_call_id.clone());
                result.push((origin, LlmMessage::ToolResult(tool_result)));
            }
            LlmMessage::User(user) => {
                settle(&mut result, &mut pending, &mut answered);
                result.push((origin, LlmMessage::User(user)));
            }
        }
    }
    settle(&mut result, &mut pending, &mut answered);
    if options.supports_images {
        leave_out_older_images(&mut result);
    }
    result
}

/// Replaces the oldest images with a note when the request would carry more than
/// `MAX_REQUEST_IMAGES` of them or more than `MAX_REQUEST_IMAGE_BYTES`. Past the count they go
/// ten at a time, so the same ones stay out from one request to the next; past the bytes, only
/// as many as must, and never the latest. A file the transcript names can be read again.
fn leave_out_older_images(messages: &mut [(usize, LlmMessage)]) {
    fn content(message: &LlmMessage) -> Option<&Vec<ContentPart>> {
        match message {
            LlmMessage::User(user) => Some(&user.content),
            LlmMessage::ToolResult(result) => Some(&result.content),
            LlmMessage::Assistant(_) => None,
        }
    }
    let sizes: Vec<usize> = messages
        .iter()
        .filter_map(|(_, message)| content(message))
        .flatten()
        .filter_map(|part| match part {
            ContentPart::Image { data, .. } => Some(data.len()),
            ContentPart::Text { .. } => None,
        })
        .collect();
    let total = sizes.len();
    let mut fitting = 0;
    let mut bytes = 0;
    for size in sizes.iter().rev() {
        if fitting > 0 && bytes + size > MAX_REQUEST_IMAGE_BYTES {
            break;
        }
        fitting += 1;
        bytes += size;
    }
    let by_count = if total > MAX_REQUEST_IMAGES { (total - MAX_REQUEST_IMAGES).div_ceil(IMAGE_DROP_STEP) * IMAGE_DROP_STEP } else { 0 };
    let dropped = by_count.max(total - fitting).min(total.saturating_sub(1));
    if dropped == 0 {
        return;
    }
    let mut seen = 0;
    for (_, message) in messages.iter_mut() {
        if seen == dropped {
            break;
        }
        let content = match message {
            LlmMessage::User(user) => &mut user.content,
            LlmMessage::ToolResult(result) => &mut result.content,
            LlmMessage::Assistant(_) => continue,
        };
        if !content.iter().any(|part| matches!(part, ContentPart::Image { .. })) {
            continue;
        }
        let mut kept = Vec::with_capacity(content.len());
        let mut previous_was_placeholder = false;
        for part in content.drain(..) {
            match part {
                ContentPart::Image { .. } if seen < dropped => {
                    seen += 1;
                    if !previous_was_placeholder {
                        kept.push(ContentPart::text(OLDER_IMAGE_PLACEHOLDER));
                    }
                    previous_was_placeholder = true;
                }
                part => {
                    previous_was_placeholder = part.as_text() == Some(OLDER_IMAGE_PLACEHOLDER);
                    kept.push(part);
                }
            }
        }
        *content = kept;
    }
}

fn replace_images(content: &[ContentPart], placeholder: &str) -> Vec<ContentPart> {
    let mut out = Vec::with_capacity(content.len());
    let mut previous_was_placeholder = false;
    for part in content {
        match part {
            ContentPart::Image { .. } => {
                if !previous_was_placeholder {
                    out.push(ContentPart::text(placeholder));
                }
                previous_was_placeholder = true;
            }
            ContentPart::Text { text } => {
                previous_was_placeholder = text == placeholder;
                out.push(part.clone());
            }
        }
    }
    out
}

/// Whether the transcript's assistant message came from this provider and model.
pub fn is_own(message: &AssistantMessage, provider: &str, model: &str) -> bool {
    message.provider == provider && message.model == model
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{ToolCall, UserMessage};
    use serde_json::json;

    fn options() -> TransformOptions<'static> {
        TransformOptions { provider: "p", model: "m", supports_images: true, normalize_tool_call_id: None }
    }

    fn assistant(provider: &str, parts: Vec<AssistantPart>) -> AssistantMessage {
        let mut message = AssistantMessage::empty(provider, "m");
        message.content = parts;
        message
    }

    #[test]
    fn foreign_thinking_becomes_text_and_own_seals_stay() {
        let own = assistant("p", vec![
            AssistantPart::Thinking { thinking: String::new(), signature: Some("sig".into()) },
            AssistantPart::Thinking { thinking: "unsigned".into(), signature: None },
            AssistantPart::ServerBlock { block: json!({ "type": "server_tool_use" }) },
        ]);
        let foreign = assistant("", vec![
            AssistantPart::Thinking { thinking: "old reasoning".into(), signature: Some("x".into()) },
            AssistantPart::Thinking { thinking: "  ".into(), signature: None },
            AssistantPart::ServerBlock { block: json!({ "type": "server_tool_use" }) },
            AssistantPart::Text { text: "hi".into() },
        ]);
        let out = transform_messages(&[LlmMessage::Assistant(foreign), LlmMessage::Assistant(own)], &options());
        let LlmMessage::Assistant(foreign) = &out[0] else { panic!() };
        assert_eq!(foreign.content, vec![AssistantPart::Text { text: "old reasoning".into() }, AssistantPart::Text { text: "hi".into() }]);
        let LlmMessage::Assistant(own) = &out[1] else { panic!() };
        assert_eq!(own.content.len(), 3);
    }

    #[test]
    fn failed_turns_are_dropped_and_orphaned_calls_get_a_result() {
        let mut failed = assistant("p", vec![AssistantPart::Text { text: "partial".into() }]);
        failed.stop_reason = StopReason::Error;
        let calls = assistant("p", vec![
            AssistantPart::ToolCall(ToolCall { id: "a".into(), name: "read".into(), arguments: json!({}) }),
            AssistantPart::ToolCall(ToolCall { id: "b".into(), name: "read".into(), arguments: json!({}) }),
        ]);
        let result_a = ToolResultMessage { tool_call_id: "a".into(), tool_name: "read".into(), content: vec![ContentPart::text("ok")], details: json!(null), is_error: false, timestamp: 0 };
        let out = transform_messages(
            &[LlmMessage::User(UserMessage::text("go")), LlmMessage::Assistant(failed), LlmMessage::Assistant(calls), LlmMessage::ToolResult(result_a), LlmMessage::User(UserMessage::text("next"))],
            &options(),
        );
        let roles: Vec<&str> = out.iter().map(|m| match m { LlmMessage::User(_) => "user", LlmMessage::Assistant(_) => "assistant", LlmMessage::ToolResult(_) => "tool" }).collect();
        assert_eq!(roles, vec!["user", "assistant", "tool", "tool", "user"]);
        let LlmMessage::ToolResult(synthetic) = &out[3] else { panic!() };
        assert_eq!((synthetic.tool_call_id.as_str(), synthetic.is_error, synthetic.text().as_str()), ("b", true, NO_RESULT_PROVIDED));
    }

    #[test]
    fn foreign_tool_call_ids_are_normalized_with_their_results() {
        let calls = assistant("other", vec![AssistantPart::ToolCall(ToolCall { id: "call|weird".into(), name: "x".into(), arguments: json!({}) })]);
        let result = ToolResultMessage { tool_call_id: "call|weird".into(), tool_name: "x".into(), content: vec![], details: json!(null), is_error: false, timestamp: 0 };
        let opts = TransformOptions { normalize_tool_call_id: Some(|id| id.replace('|', "_")), ..options() };
        let out = transform_messages(&[LlmMessage::Assistant(calls), LlmMessage::ToolResult(result)], &opts);
        let LlmMessage::Assistant(a) = &out[0] else { panic!() };
        assert_eq!(a.tool_calls()[0].id, "call_weird");
        let LlmMessage::ToolResult(r) = &out[1] else { panic!() };
        assert_eq!(r.tool_call_id, "call_weird");
    }

    #[test]
    fn a_request_carries_only_its_latest_images() {
        let image = |data: &str| ContentPart::Image { data: data.into(), mime_type: "image/png".into() };
        let shots = |count: usize| -> Vec<LlmMessage> {
            (0..count).map(|index| LlmMessage::User(UserMessage { content: vec![ContentPart::text(format!("shot {index}")), image("AAAA")], timestamp: 0 })).collect()
        };
        let images = |messages: &[LlmMessage]| -> usize {
            messages
                .iter()
                .map(|message| match message {
                    LlmMessage::User(user) => user.content.iter().filter(|part| matches!(part, ContentPart::Image { .. })).count(),
                    _ => 0,
                })
                .sum()
        };
        assert_eq!(images(&transform_messages(&shots(20), &options())), 20);
        let out = transform_messages(&shots(21), &options());
        assert_eq!(images(&out), 11, "ten leave at once");
        let LlmMessage::User(first) = &out[0] else { panic!() };
        assert_eq!(first.content, vec![ContentPart::text("shot 0"), ContentPart::text(OLDER_IMAGE_PLACEHOLDER)]);
        assert_eq!(images(&transform_messages(&shots(30), &options())), 20, "the same ten stay out until ten more come");
        assert_eq!(images(&transform_messages(&shots(31), &options())), 11);

        let large = "A".repeat(7 * 1024 * 1024);
        let heavy: Vec<LlmMessage> = (0..3).map(|_| LlmMessage::User(UserMessage { content: vec![image(&large)], timestamp: 0 })).collect();
        assert_eq!(images(&transform_messages(&heavy, &options())), 2, "as many as fit 16 MB");
        let alone = [LlmMessage::User(UserMessage { content: vec![image(&"A".repeat(17 * 1024 * 1024))], timestamp: 0 })];
        assert_eq!(images(&transform_messages(&alone, &options())), 1, "never the latest");
        let opts = TransformOptions { supports_images: false, ..options() };
        assert!(transform_messages(&shots(25), &opts).iter().all(|message| !matches!(message, LlmMessage::User(user) if user.content.iter().any(|part| part.as_text() == Some(OLDER_IMAGE_PLACEHOLDER)))));
    }

    #[test]
    fn images_become_one_note_for_a_text_only_model() {
        let user = UserMessage { content: vec![ContentPart::text("see"), ContentPart::Image { data: "a".into(), mime_type: "image/png".into() }, ContentPart::Image { data: "b".into(), mime_type: "image/png".into() }], timestamp: 0 };
        let opts = TransformOptions { supports_images: false, ..options() };
        let out = transform_messages(&[LlmMessage::User(user)], &opts);
        let LlmMessage::User(u) = &out[0] else { panic!() };
        assert_eq!(u.content, vec![ContentPart::text("see"), ContentPart::text(NON_VISION_USER_IMAGE_PLACEHOLDER)]);
    }
}
