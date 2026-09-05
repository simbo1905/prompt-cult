//! Translate between OAI Responses API and Mistral Chat Completions API
//! request/response formats.
//!
//! Mistral's Chat Completions API is OpenAI-compatible: tools pass through
//! unchanged, `input` message items flatten to `messages`, and reasoning
//! fields are dropped (Mistral has no Responses-style reasoning effort).
//!
//! ## Field mapping
//!
//! ```text
//! OAI Responses              →  Mistral Chat Completions
//! ─────────────────────────────  ────────────────────────────────────
//! instructions               →  messages[0] {role:"system", content:...}
//! input[].type == "message" →  messages[] {role, content}
//! input[].type == "function_call"          → assistant {tool_calls:[...]}
//! input[].type == "function_call_output"   → {role:"tool", tool_call_id, content}
//! tools[]                    →  tools[]  (identical OpenAI function schema)
//! max_output_tokens          →  max_tokens
//! stream                     →  stream
//! reasoning_effort/summary  →  (dropped)
//! ```

use serde_json::Map;
use serde_json::Value;
use serde_json::json;
use uuid::Uuid;

/// Convert an OAI Responses API request body into a Mistral Chat Completions body.
pub(crate) fn oai_to_mistral(oai: &Value) -> Value {
    let model = oai["model"].as_str().unwrap_or("");
    let is_stream = oai["stream"].as_bool().unwrap_or(false);
    let max_tokens = oai["max_output_tokens"].as_u64().unwrap_or(16384);

    let instructions = oai["instructions"].as_str();
    let input = oai["input"].as_array();

    let (system_from_input, messages) = convert_input_to_messages(input);
    // The OAI Responses API treats top-level `instructions` as a prepended
    // system turn; system/developer messages embedded in `input` are
    // additional context, not alternatives. Concatenate so neither is lost.
    let system = match (instructions, system_from_input.as_deref()) {
        (Some(a), Some(b)) => Some(format!("{a}\n\n{b}")),
        (Some(a), None) => Some(a.to_string()),
        (None, b) => b.map(str::to_string),
    };

    let tools = convert_tools(oai["tools"].as_array());

    // Build the final messages array: optional system message first, then the
    // converted conversation items.
    let mut all_messages: Vec<Value> = Vec::new();
    if let Some(sys) = system {
        all_messages.push(json!({"role": "system", "content": sys}));
    }
    all_messages.extend(messages);

    let mut body = json!({
        "model": model,
        "max_tokens": max_tokens,
        "messages": all_messages,
        "stream": is_stream,
    });

    if !tools.is_empty() {
        body["tools"] = Value::Array(tools);
    }

    body
}

/// Convert a non-streaming Mistral Chat Completions response into OAI Responses format.
pub(crate) fn mistral_response_to_oai(mistral: &Value, model: &str) -> Value {
    let resp_id = format!("resp_{}", Uuid::new_v4().simple());
    let created_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());

    let mut output = Vec::new();

    // Determine terminal status from the first choice's finish_reason.
    // Mistral's `length` means the output was truncated at `max_tokens`; the
    // OAI Responses API represents this as `status: "incomplete"` with
    // `incomplete_details.reason: "max_output_tokens"`.
    let finish_reason = mistral["choices"]
        .as_array()
        .and_then(|choices| choices.first())
        .and_then(|choice| choice["finish_reason"].as_str())
        .unwrap_or("");
    let is_length = finish_reason == "length";
    let status = if is_length { "incomplete" } else { "completed" };
    let incomplete_details: Option<Value> = if is_length {
        Some(json!({"reason": "max_output_tokens"}))
    } else {
        None
    };

    if let Some(choices) = mistral["choices"].as_array() {
        for choice in choices {
            let message = &choice["message"];
            let role = message["role"].as_str().unwrap_or("assistant");

            // Text content → message output item.
            if let Some(content) = message["content"].as_str()
                && !content.is_empty()
            {
                let msg_id = format!("msg_{}", Uuid::new_v4().simple());
                output.push(json!({
                    "id": msg_id,
                    "type": "message",
                    "status": "completed",
                    "role": role,
                    "content": [{"type": "output_text", "text": content, "annotations": []}],
                }));
            }

            // Tool calls → function_call output items.
            if let Some(tool_calls) = message["tool_calls"].as_array() {
                for tc in tool_calls {
                    let call_id = tc["id"].as_str().unwrap_or("").to_string();
                    let name = tc["function"]["name"].as_str().unwrap_or("");
                    let arguments = tc["function"]["arguments"].as_str().unwrap_or("{}");
                    output.push(json!({
                        "id": call_id,
                        "type": "function_call",
                        "status": "completed",
                        "name": name,
                        "call_id": call_id,
                        "arguments": arguments,
                    }));
                }
            }
        }
    }

    let usage = &mistral["usage"];
    let input_tokens = usage["prompt_tokens"].as_u64().unwrap_or(0);
    let output_tokens = usage["completion_tokens"].as_u64().unwrap_or(0);

    json!({
        "id": resp_id,
        "object": "response",
        "created_at": created_at,
        "status": status,
        "model": model,
        "output": output,
        "usage": {
            "input_tokens": input_tokens,
            "input_tokens_details": {"cached_tokens": 0},
            "output_tokens": output_tokens,
            "output_tokens_details": {"reasoning_tokens": 0},
            "total_tokens": input_tokens + output_tokens,
        },
        "incomplete_details": incomplete_details,
        "error": null,
    })
}

/// Convert OAI `input` array to (optional system prompt, Mistral messages).
fn convert_input_to_messages(input: Option<&Vec<Value>>) -> (Option<String>, Vec<Value>) {
    let Some(items) = input else {
        return (None, vec![]);
    };

    let mut messages: Vec<Value> = Vec::new();
    let mut system: Option<String> = None;

    for item in items {
        let itype = item["type"].as_str().unwrap_or("message");
        let role = item["role"].as_str().unwrap_or("user");

        match itype {
            "message" => {
                let text = extract_message_text(item);

                if role == "system" || role == "developer" {
                    system = Some(match system {
                        Some(existing) => format!("{existing}\n\n{text}"),
                        None => text,
                    });
                } else {
                    messages.push(json!({"role": role, "content": text}));
                }
            }
            "function_call" => {
                let call_id = item["call_id"]
                    .as_str()
                    .or_else(|| item["id"].as_str())
                    .unwrap_or("call_unknown")
                    .to_string();
                let name = item["name"].as_str().unwrap_or("");
                let arguments = item["arguments"].as_str().unwrap_or("{}");

                let tool_call = json!({
                    "id": call_id,
                    "type": "function",
                    "function": {
                        "name": name,
                        "arguments": arguments,
                    },
                });

                // Append to existing assistant message or create new one.
                if let Some(last) = messages.last_mut()
                    && last["role"].as_str() == Some("assistant")
                {
                    let tool_calls = last
                        .as_object_mut()
                        .unwrap_or_else(|| unreachable!())
                        .entry("tool_calls".to_string())
                        .or_insert(Value::Array(vec![]));
                    if let Some(arr) = tool_calls.as_array_mut() {
                        arr.push(tool_call);
                    }
                    continue;
                }
                messages.push(json!({
                    "role": "assistant",
                    "content": "",
                    "tool_calls": [tool_call],
                }));
            }
            "function_call_output" => {
                let call_id = item["call_id"].as_str().unwrap_or("");
                let output = item["output"].as_str().unwrap_or("");
                messages.push(json!({
                    "role": "tool",
                    "tool_call_id": call_id,
                    "content": output,
                }));
            }
            _ => {}
        }
    }

    (system, messages)
}

/// Extract text content from an OAI message item.
fn extract_message_text(item: &Value) -> String {
    match &item["content"] {
        Value::String(s) => s.clone(),
        Value::Array(parts) => {
            let mut texts = Vec::new();
            for part in parts {
                if let Some("input_text" | "text" | "output_text") = part["type"].as_str()
                    && let Some(t) = part["text"].as_str()
                {
                    texts.push(t.to_string());
                }
            }
            texts.join("\n")
        }
        _ => String::new(),
    }
}

/// Convert OAI Responses tool definitions to Mistral (OpenAI-compatible) tool format.
fn convert_tools(tools: Option<&Vec<Value>>) -> Vec<Value> {
    let Some(tools) = tools else {
        return vec![];
    };

    let mut result = Vec::new();
    for tool in tools {
        let (name, description, parameters) = if tool["type"].as_str() == Some("function") {
            if let Some(func) = tool.get("function") {
                (
                    func["name"].as_str().unwrap_or(""),
                    func["description"].as_str().unwrap_or(""),
                    func.get("parameters")
                        .cloned()
                        .unwrap_or(json!({"type": "object", "properties": {}})),
                )
            } else {
                (
                    tool["name"].as_str().unwrap_or(""),
                    tool["description"].as_str().unwrap_or(""),
                    tool.get("parameters")
                        .cloned()
                        .unwrap_or(json!({"type": "object", "properties": {}})),
                )
            }
        } else {
            (
                tool["name"].as_str().unwrap_or(""),
                tool["description"].as_str().unwrap_or(""),
                tool.get("parameters")
                    .cloned()
                    .unwrap_or_else(|| Value::Object(Map::new())),
            )
        };

        if name.is_empty() {
            continue;
        }

        result.push(json!({
            "type": "function",
            "function": {
                "name": name,
                "description": description,
                "parameters": parameters,
            },
        }));
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn instructions_and_input_system_message_are_concatenated() {
        let oai = json!({
            "model": "m",
            "instructions": "Top-level instructions.",
            "input": [
                {"type": "message", "role": "developer", "content": "Input-embedded context."},
                {"type": "message", "role": "user", "content": "Hello"}
            ]
        });
        let result = oai_to_mistral(&oai);
        assert_eq!(result["messages"][0]["role"], "system");
        assert_eq!(
            result["messages"][0]["content"],
            "Top-level instructions.\n\nInput-embedded context."
        );
        assert_eq!(result["messages"][1]["role"], "user");
    }

    #[test]
    fn input_system_message_survives_without_instructions() {
        let oai = json!({
            "model": "m",
            "input": [
                {"type": "message", "role": "system", "content": "Only embedded."},
                {"type": "message", "role": "user", "content": "Hello"}
            ]
        });
        let result = oai_to_mistral(&oai);
        assert_eq!(result["messages"][0]["content"], "Only embedded.");
    }

    #[test]
    fn test_basic_request_translation() {
        let oai = json!({
            "model": "zai-glm-5-2",
            "stream": true,
            "instructions": "You are helpful.",
            "input": [
                {"type": "message", "role": "user", "content": "Hello"}
            ],
            "tools": [{
                "type": "function",
                "name": "shell",
                "description": "Run a shell command",
                "parameters": {"type": "object", "properties": {"command": {"type": "string"}}}
            }],
            "max_output_tokens": 8192,
        });

        let result = oai_to_mistral(&oai);

        assert_eq!(result["model"], "zai-glm-5-2");
        assert_eq!(result["max_tokens"], 8192);
        assert_eq!(result["stream"], true);
        // System message is first.
        assert_eq!(result["messages"][0]["role"], "system");
        assert_eq!(result["messages"][0]["content"], "You are helpful.");
        assert_eq!(result["messages"][1]["role"], "user");
        assert_eq!(result["messages"][1]["content"], "Hello");
        assert_eq!(result["tools"][0]["function"]["name"], "shell");
        assert_eq!(
            result["tools"][0]["function"]["parameters"]["properties"]["command"]["type"],
            "string"
        );
    }

    #[test]
    fn test_function_call_roundtrip() {
        let oai = json!({
            "model": "zai-glm-5-2",
            "stream": true,
            "input": [
                {"type": "message", "role": "user", "content": "List files"},
                {"type": "function_call", "call_id": "call_123", "name": "shell", "arguments": "{\"command\":\"ls\"}"},
                {"type": "function_call_output", "call_id": "call_123", "output": "file1.txt\nfile2.txt"}
            ],
            "max_output_tokens": 16384,
        });

        let result = oai_to_mistral(&oai);
        let messages = result["messages"]
            .as_array()
            .unwrap_or_else(|| unreachable!());

        assert_eq!(messages.len(), 3);
        assert_eq!(messages[0]["role"], "user");
        assert_eq!(messages[1]["role"], "assistant");
        assert_eq!(messages[1]["tool_calls"][0]["id"], "call_123");
        assert_eq!(messages[1]["tool_calls"][0]["type"], "function");
        assert_eq!(messages[1]["tool_calls"][0]["function"]["name"], "shell");
        assert_eq!(
            messages[1]["tool_calls"][0]["function"]["arguments"],
            "{\"command\":\"ls\"}"
        );
        assert_eq!(messages[2]["role"], "tool");
        assert_eq!(messages[2]["tool_call_id"], "call_123");
        assert_eq!(messages[2]["content"], "file1.txt\nfile2.txt");
    }

    #[test]
    fn test_non_streaming_response_translation() {
        let mistral = json!({
            "id": "chatcmpl-abc",
            "model": "zai-glm-5-2",
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": "Hello back!",
                    "tool_calls": [{
                        "id": "call_456",
                        "type": "function",
                        "function": {
                            "name": "shell",
                            "arguments": "{\"command\":\"pwd\"}"
                        }
                    }]
                },
                "finish_reason": "stop"
            }],
            "usage": {
                "prompt_tokens": 10,
                "completion_tokens": 5,
                "total_tokens": 15
            }
        });

        let result = mistral_response_to_oai(&mistral, "zai-glm-5-2");

        assert_eq!(result["object"], "response");
        assert_eq!(result["status"], "completed");
        assert_eq!(result["model"], "zai-glm-5-2");
        let output = result["output"]
            .as_array()
            .unwrap_or_else(|| unreachable!());
        assert_eq!(output.len(), 2);
        assert_eq!(output[0]["type"], "message");
        assert_eq!(output[0]["content"][0]["text"], "Hello back!");
        assert_eq!(output[1]["type"], "function_call");
        assert_eq!(output[1]["name"], "shell");
        assert_eq!(output[1]["call_id"], "call_456");
        assert_eq!(result["usage"]["input_tokens"], 10);
        assert_eq!(result["usage"]["output_tokens"], 5);
    }

    #[test]
    fn length_finish_reason_maps_to_incomplete() {
        let mistral = json!({
            "choices": [{
                "message": {"role": "assistant", "content": "trunc"},
                "finish_reason": "length"
            }],
            "usage": {"prompt_tokens": 1, "completion_tokens": 2}
        });
        let result = mistral_response_to_oai(&mistral, "m");
        assert_eq!(result["status"], "incomplete");
        assert_eq!(result["incomplete_details"]["reason"], "max_output_tokens");
    }
}
