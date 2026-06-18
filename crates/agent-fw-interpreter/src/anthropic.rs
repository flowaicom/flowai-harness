//! Anthropic Messages API interpreter with SSE streaming and tool loop.
//!
//! Implements `ChatInterpreter` by calling the Anthropic Messages API with
//! `stream: true` and parsing the resulting SSE events into `StreamPart`s.
//!
//! # Tool Loop
//!
//! When the model emits tool_use blocks and a `ToolDispatcher` is configured,
//! the interpreter automatically dispatches tool calls, appends results, and
//! calls the API again. This continues until the model emits `end_turn` or
//! a maximum iteration limit is reached.
//!
//! # Extended Thinking
//!
//! When the `AgentBlueprint` has `thinking_budget` set, the request includes
//! `thinking: { type: "enabled", budget_tokens: N }` and thinking deltas
//! are mapped to `StreamPart::Reasoning`.

use agent_fw_agent::{
    anthropic_reasoning_params, AgentBlueprint, ChatInterpreter, ChatMessage, ChatProgram,
    ChatRole, ToolDispatcher,
};
use agent_fw_algebra::CancellationToken;
use agent_fw_core::{FinishReason, StreamPart, TokenUsage};
use async_trait::async_trait;
use std::pin::Pin;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tracing;

/// Configuration for the Anthropic API.
#[derive(Debug, Clone)]
pub struct AnthropicConfig {
    pub api_key: String,
    pub base_url: String,
    pub api_version: String,
    pub max_tokens: u32,
    pub default_model: String,
}

impl AnthropicConfig {
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            base_url: "https://api.anthropic.com".into(),
            api_version: "2023-06-01".into(),
            max_tokens: 8192,
            default_model: "claude-sonnet-4-20250514".into(),
        }
    }

    pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = url.into();
        self
    }
}

/// Maximum number of tool-loop iterations to prevent infinite loops.
const MAX_TOOL_ITERATIONS: usize = 25;

/// Anthropic Messages API ChatInterpreter.
///
/// Streams responses via SSE and supports agentic tool loops.
pub struct AnthropicInterpreter {
    client: reqwest::Client,
    config: AnthropicConfig,
    blueprint: AgentBlueprint,
    tool_dispatcher: Option<Arc<dyn ToolDispatcher>>,
}

impl AnthropicInterpreter {
    pub fn new(config: AnthropicConfig, blueprint: AgentBlueprint) -> Self {
        Self {
            client: reqwest::Client::new(),
            config,
            blueprint,
            tool_dispatcher: None,
        }
    }

    pub fn with_tool_dispatcher(mut self, dispatcher: Arc<dyn ToolDispatcher>) -> Self {
        self.tool_dispatcher = Some(dispatcher);
        self
    }

    /// Build the JSON request body for the Messages API.
    fn build_request_body(
        &self,
        program: &ChatProgram,
        extra_messages: &[serde_json::Value],
    ) -> serde_json::Value {
        let model = if !program.model().as_str().is_empty() {
            program.model().as_str()
        } else if !self.blueprint.model.as_str().is_empty() {
            self.blueprint.model.as_str()
        } else {
            &self.config.default_model
        };

        let max_tokens = self.blueprint.max_tokens.unwrap_or(self.config.max_tokens);

        // Build messages array from conversation
        let mut messages: Vec<serde_json::Value> = program
            .conversation()
            .messages()
            .iter()
            .filter(|m| m.role != ChatRole::System)
            .map(|m| message_to_json(m))
            .collect();

        // Append any extra messages (tool results for agentic loop)
        messages.extend_from_slice(extra_messages);

        let system_text = program.system_prompt().as_str();
        if self.blueprint.prompt_caching {
            apply_message_cache_control(&mut messages);
        }

        let mut body = serde_json::json!({
            "model": model,
            "max_tokens": max_tokens,
            "stream": true,
            "messages": messages,
        });

        if !system_text.is_empty() {
            if self.blueprint.prompt_caching {
                body["system"] = serde_json::json!([{
                    "type": "text",
                    "text": system_text,
                    "cache_control": anthropic_cache_breakpoint(),
                }]);
            } else {
                body["system"] = serde_json::json!(system_text);
            }
        }

        // Tool definitions
        if let Some(ref dispatcher) = self.tool_dispatcher {
            let mut tools: Vec<serde_json::Value> = dispatcher
                .tool_definitions()
                .into_iter()
                .map(|td| {
                    serde_json::json!({
                        "name": td.name,
                        "description": td.description,
                        "input_schema": td.input_schema,
                    })
                })
                .collect();

            if !tools.is_empty() {
                if self.blueprint.prompt_caching {
                    apply_tool_cache_control(&mut tools);
                }
                body["tools"] = serde_json::json!(tools);
            }
        }

        if let Some(params) = anthropic_reasoning_params(
            body["model"].as_str().unwrap_or_default(),
            true,
            self.blueprint.thinking_budget.unwrap_or(0),
            self.blueprint.reasoning_effort,
        ) {
            if let Some(object) = params.as_object() {
                for (key, value) in object {
                    body[key] = value.clone();
                }
            }
        }

        body
    }

    /// Send a streaming request and parse SSE events.
    ///
    /// Returns the accumulated tool_use blocks (if any), the text content,
    /// the stop_reason, and final usage.
    async fn stream_response(
        &self,
        body: serde_json::Value,
        tx: &mpsc::Sender<StreamPart>,
        cancel: &CancellationToken,
    ) -> Result<StreamResult, StreamError> {
        let url = format!("{}/v1/messages", self.config.base_url);

        let response = self
            .client
            .post(&url)
            .header("x-api-key", &self.config.api_key)
            .header("anthropic-version", &self.config.api_version)
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| StreamError::Request(e.to_string()))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(StreamError::Api(format!("{status}: {body}")));
        }

        let mut stream_result = StreamResult::default();
        let mut current_block_type: Option<String> = None;
        let mut current_tool_name = String::new();
        let mut current_tool_id = String::new();
        let mut current_tool_json = String::new();

        // Read SSE events line by line
        let mut bytes_stream = response.bytes_stream();
        let mut buffer = String::new();

        use tokio_stream::StreamExt;
        while let Some(chunk) = bytes_stream.next().await {
            if cancel.is_cancelled() {
                return Err(StreamError::Cancelled);
            }

            let chunk = chunk.map_err(|e| StreamError::Request(e.to_string()))?;
            buffer.push_str(&String::from_utf8_lossy(&chunk));

            // Process complete SSE events from buffer
            while let Some(pos) = buffer.find("\n\n") {
                let event_text = buffer[..pos].to_string();
                buffer = buffer[pos + 2..].to_string();

                let (event_type, data) = parse_sse_event(&event_text);
                if data.is_empty() {
                    continue;
                }

                let parsed: serde_json::Value = match serde_json::from_str(&data) {
                    Ok(v) => v,
                    Err(_) => continue,
                };

                match event_type.as_str() {
                    "message_start" => {
                        if let Some(usage) = parsed.get("message").and_then(|m| m.get("usage")) {
                            if let Some(input) = usage.get("input_tokens").and_then(|v| v.as_u64())
                            {
                                stream_result.usage.prompt_tokens = input;
                            }
                            if let Some(created) = usage
                                .get("cache_creation_input_tokens")
                                .and_then(|v| v.as_u64())
                            {
                                stream_result.usage.prompt_tokens =
                                    stream_result.usage.prompt_tokens.saturating_add(created);
                                stream_result.usage.cache_creation_input_tokens = created;
                            }
                            if let Some(cached) = usage
                                .get("cache_read_input_tokens")
                                .and_then(|v| v.as_u64())
                            {
                                stream_result.usage.cache_read_input_tokens = cached;
                            }
                        }
                    }
                    "content_block_start" => {
                        let block_type = parsed
                            .get("content_block")
                            .and_then(|b| b.get("type"))
                            .and_then(|t| t.as_str())
                            .unwrap_or("")
                            .to_string();

                        if block_type == "tool_use" {
                            if let Some(block) = parsed.get("content_block") {
                                current_tool_name = block
                                    .get("name")
                                    .and_then(|n| n.as_str())
                                    .unwrap_or("")
                                    .to_string();
                                current_tool_id = block
                                    .get("id")
                                    .and_then(|i| i.as_str())
                                    .unwrap_or("")
                                    .to_string();
                                current_tool_json.clear();
                            }
                        }

                        current_block_type = Some(block_type);
                    }
                    "content_block_delta" => {
                        let delta = parsed.get("delta");
                        match current_block_type.as_deref() {
                            Some("text") => {
                                if let Some(text) =
                                    delta.and_then(|d| d.get("text")).and_then(|t| t.as_str())
                                {
                                    stream_result.text.push_str(text);
                                    let _ = tx.send(StreamPart::text(text)).await;
                                }
                            }
                            Some("thinking") => {
                                if let Some(text) = delta
                                    .and_then(|d| d.get("thinking"))
                                    .and_then(|t| t.as_str())
                                {
                                    let _ = tx.send(StreamPart::reasoning(text)).await;
                                }
                            }
                            Some("tool_use") => {
                                if let Some(json) = delta
                                    .and_then(|d| d.get("partial_json"))
                                    .and_then(|j| j.as_str())
                                {
                                    current_tool_json.push_str(json);
                                }
                            }
                            _ => {}
                        }
                    }
                    "content_block_stop" => {
                        if current_block_type.as_deref() == Some("tool_use") {
                            let input: serde_json::Value = serde_json::from_str(&current_tool_json)
                                .unwrap_or_else(|e| {
                                    tracing::warn!(
                                        tool = %current_tool_name,
                                        error = %e,
                                        json_len = current_tool_json.len(),
                                        "Malformed tool input JSON from API, using null"
                                    );
                                    serde_json::Value::Null
                                });

                            let _ = tx
                                .send(StreamPart::tool_call(
                                    &current_tool_id,
                                    &current_tool_name,
                                    input.clone(),
                                ))
                                .await;

                            stream_result.tool_use_blocks.push(ToolUseBlock {
                                id: current_tool_id.clone(),
                                name: current_tool_name.clone(),
                                input,
                            });

                            current_tool_json.clear();
                        }
                        current_block_type = None;
                    }
                    "message_delta" => {
                        if let Some(delta) = parsed.get("delta") {
                            if let Some(reason) = delta.get("stop_reason").and_then(|r| r.as_str())
                            {
                                stream_result.stop_reason = reason.to_string();
                            }
                        }
                        if let Some(usage) = parsed.get("usage") {
                            if let Some(output) =
                                usage.get("output_tokens").and_then(|v| v.as_u64())
                            {
                                stream_result.usage.completion_tokens = output;
                            }
                        }
                    }
                    "message_stop" | "error" => {
                        if event_type == "error" {
                            let msg = parsed
                                .get("error")
                                .and_then(|e| e.get("message"))
                                .and_then(|m| m.as_str())
                                .unwrap_or("Unknown API error");
                            return Err(StreamError::Api(msg.to_string()));
                        }
                        break;
                    }
                    _ => {}
                }
            }
        }

        Ok(stream_result)
    }
}

fn message_to_json(msg: &ChatMessage) -> serde_json::Value {
    let role = match msg.role {
        ChatRole::User => "user",
        ChatRole::Assistant => "assistant",
        ChatRole::System => "user", // system messages handled separately
    };
    serde_json::json!({
        "role": role,
        "content": msg.content,
    })
}

fn anthropic_cache_breakpoint() -> serde_json::Value {
    serde_json::json!({ "type": "ephemeral" })
}

fn set_content_cache_control(block: &mut serde_json::Value, value: Option<serde_json::Value>) {
    let Some(object) = block.as_object_mut() else {
        return;
    };

    object.remove("cache_control");
    let cacheable = matches!(
        object.get("type").and_then(serde_json::Value::as_str),
        Some("text" | "image" | "tool_result" | "document")
    );
    if cacheable {
        if let Some(value) = value {
            object.insert("cache_control".to_string(), value);
        }
    }
}

fn apply_message_cache_control(messages: &mut [serde_json::Value]) {
    for message in messages.iter_mut() {
        if let Some(blocks) = message
            .get_mut("content")
            .and_then(serde_json::Value::as_array_mut)
        {
            for block in blocks.iter_mut() {
                set_content_cache_control(block, None);
            }
        }
    }

    if let Some(last_message) = messages.last_mut() {
        if let Some(content) = last_message.get_mut("content") {
            if let Some(text) = content.as_str() {
                if !text.is_empty() {
                    *content = serde_json::json!([{
                        "type": "text",
                        "text": text,
                        "cache_control": anthropic_cache_breakpoint(),
                    }]);
                }
            } else if let Some(blocks) = content.as_array_mut() {
                if let Some(last_block) = blocks.last_mut() {
                    set_content_cache_control(last_block, Some(anthropic_cache_breakpoint()));
                }
            }
        }
    }
}

fn apply_tool_cache_control(tools: &mut [serde_json::Value]) {
    for tool in tools.iter_mut() {
        if let Some(object) = tool.as_object_mut() {
            object.remove("cache_control");
        }
    }

    if let Some(last_tool) = tools.last_mut() {
        if let Some(object) = last_tool.as_object_mut() {
            object.insert("cache_control".to_string(), anthropic_cache_breakpoint());
        }
    }
}

fn parse_sse_event(text: &str) -> (String, String) {
    let mut event_type = "message".to_string();
    let mut data_lines = Vec::new();

    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("event: ") {
            event_type = rest.trim().to_string();
        } else if let Some(rest) = line.strip_prefix("data: ") {
            data_lines.push(rest);
        }
    }

    (event_type, data_lines.join("\n"))
}

#[derive(Debug, Default)]
struct StreamResult {
    text: String,
    tool_use_blocks: Vec<ToolUseBlock>,
    stop_reason: String,
    usage: TokenUsage,
}

#[derive(Debug, Clone)]
struct ToolUseBlock {
    id: String,
    name: String,
    input: serde_json::Value,
}

#[derive(Debug)]
enum StreamError {
    Request(String),
    Api(String),
    Cancelled,
}

#[async_trait]
impl ChatInterpreter for AnthropicInterpreter {
    fn interpret(
        &self,
        program: ChatProgram,
        cancel: CancellationToken,
    ) -> Pin<Box<dyn futures::Stream<Item = StreamPart> + Send>> {
        let (tx, rx) = mpsc::channel::<StreamPart>(64);

        let client = self.client.clone();
        let config = self.config.clone();
        let blueprint = self.blueprint.clone();
        let tool_dispatcher = self.tool_dispatcher.clone();

        // Build a self-contained interpreter for the spawned task
        let interpreter = AnthropicInterpreter {
            client,
            config,
            blueprint,
            tool_dispatcher,
        };

        tokio::spawn(async move {
            let mut extra_messages: Vec<serde_json::Value> = Vec::new();

            for _iteration in 0..MAX_TOOL_ITERATIONS {
                if cancel.is_cancelled() {
                    let _ = tx.send(StreamPart::error("Cancelled")).await;
                    break;
                }

                let body = interpreter.build_request_body(&program, &extra_messages);

                let result = interpreter.stream_response(body, &tx, &cancel).await;

                match result {
                    Ok(stream_result) => {
                        // If the model wants to use tools and we have a dispatcher
                        if stream_result.stop_reason == "tool_use"
                            && !stream_result.tool_use_blocks.is_empty()
                        {
                            // SAFETY: We only enter this branch when tool_use blocks exist.
                            // If no dispatcher is configured, skip the tool loop.
                            let Some(dispatcher) = interpreter.tool_dispatcher.as_ref() else {
                                let _ = tx.send(StreamPart::error(
                                    "Model requested tool_use but no ToolDispatcher is configured"
                                )).await;
                                break;
                            };

                            // Build assistant message with tool_use content blocks
                            let mut assistant_content: Vec<serde_json::Value> = Vec::new();
                            if !stream_result.text.is_empty() {
                                assistant_content.push(serde_json::json!({
                                    "type": "text",
                                    "text": stream_result.text,
                                }));
                            }
                            for tool_block in &stream_result.tool_use_blocks {
                                assistant_content.push(serde_json::json!({
                                    "type": "tool_use",
                                    "id": tool_block.id,
                                    "name": tool_block.name,
                                    "input": tool_block.input,
                                }));
                            }

                            extra_messages.push(serde_json::json!({
                                "role": "assistant",
                                "content": assistant_content,
                            }));

                            // Dispatch tools and build tool_result message
                            let mut tool_results: Vec<serde_json::Value> = Vec::new();
                            for tool_block in &stream_result.tool_use_blocks {
                                let call_result = dispatcher
                                    .dispatch(
                                        &tool_block.name,
                                        &tool_block.id,
                                        tool_block.input.clone(),
                                    )
                                    .await;

                                let _ = tx
                                    .send(StreamPart::tool_result(
                                        &call_result.tool_use_id,
                                        &tool_block.name,
                                        tool_block.input.clone(),
                                        call_result.content.clone(),
                                    ))
                                    .await;

                                tool_results.push(serde_json::json!({
                                    "type": "tool_result",
                                    "tool_use_id": tool_block.id,
                                    "content": serde_json::to_string(&call_result.content)
                                        .unwrap_or_default(),
                                    "is_error": call_result.is_error,
                                }));
                            }

                            extra_messages.push(serde_json::json!({
                                "role": "user",
                                "content": tool_results,
                            }));

                            // Continue the loop for the next API call
                            continue;
                        }

                        // No more tool calls — emit finish
                        let reason = match stream_result.stop_reason.as_str() {
                            "end_turn" | "stop" => FinishReason::Stop,
                            "max_tokens" => FinishReason::Length,
                            "tool_use" => FinishReason::ToolCalls,
                            _ => FinishReason::Stop,
                        };

                        let _ = tx
                            .send(StreamPart::finish(reason, stream_result.usage))
                            .await;
                        break;
                    }
                    Err(StreamError::Cancelled) => {
                        let _ = tx.send(StreamPart::error("Cancelled")).await;
                        break;
                    }
                    Err(StreamError::Request(msg) | StreamError::Api(msg)) => {
                        let _ = tx.send(StreamPart::error(msg)).await;
                        let _ = tx
                            .send(StreamPart::finish(FinishReason::Stop, TokenUsage::ZERO))
                            .await;
                        break;
                    }
                }
            }
        });

        Box::pin(ReceiverStream::new(rx))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use agent_fw_agent::{ToolCallResult, ToolDefinition};
    use async_trait::async_trait;

    struct TestDispatcher;

    #[async_trait]
    impl ToolDispatcher for TestDispatcher {
        fn tool_definitions(&self) -> Vec<ToolDefinition> {
            vec![ToolDefinition {
                name: "lookup".to_string(),
                description: "Look up a value".to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {},
                }),
            }]
        }

        async fn dispatch(
            &self,
            _tool_name: &str,
            tool_use_id: &str,
            _input: serde_json::Value,
        ) -> ToolCallResult {
            ToolCallResult::success(tool_use_id, serde_json::json!({"ok": true}))
        }
    }

    #[test]
    fn parse_sse_event_basic() {
        let input = "event: content_block_delta\ndata: {\"type\":\"delta\"}";
        let (event, data) = parse_sse_event(input);
        assert_eq!(event, "content_block_delta");
        assert_eq!(data, "{\"type\":\"delta\"}");
    }

    #[test]
    fn parse_sse_event_no_event_line() {
        let input = "data: {\"hello\":\"world\"}";
        let (event, data) = parse_sse_event(input);
        assert_eq!(event, "message");
        assert_eq!(data, "{\"hello\":\"world\"}");
    }

    #[test]
    fn config_defaults() {
        let config = AnthropicConfig::new("test-key");
        assert_eq!(config.base_url, "https://api.anthropic.com");
        assert_eq!(config.api_version, "2023-06-01");
        assert_eq!(config.max_tokens, 8192);
        assert_eq!(config.default_model, "claude-sonnet-4-20250514");
    }

    #[test]
    fn config_custom_base_url() {
        let config = AnthropicConfig::new("key").with_base_url("http://localhost:8080");
        assert_eq!(config.base_url, "http://localhost:8080");
    }

    #[test]
    fn message_to_json_user() {
        let msg = ChatMessage::user("Hello");
        let json = message_to_json(&msg);
        assert_eq!(json["role"], "user");
        assert_eq!(json["content"], "Hello");
    }

    #[test]
    fn message_to_json_assistant() {
        let msg = ChatMessage::assistant("Hi there");
        let json = message_to_json(&msg);
        assert_eq!(json["role"], "assistant");
        assert_eq!(json["content"], "Hi there");
    }

    #[test]
    fn build_request_body_basic() {
        use agent_fw_agent::{parse_conversation, ChatMessage, ModelId};
        use agent_fw_core::tenant::TenantContext;

        let config = AnthropicConfig::new("test-key");
        let blueprint =
            AgentBlueprint::new(ModelId::new("claude-sonnet-4-20250514"), "You are helpful");
        let interpreter = AnthropicInterpreter::new(config, blueprint);

        let conv = parse_conversation(vec![
            ChatMessage::system("Be helpful"),
            ChatMessage::user("Hello"),
        ])
        .unwrap();

        let tenant = TenantContext::new(agent_fw_core::id::TenantId::new("t1").unwrap());
        let program = ChatProgram::new(conv, ModelId::new("claude-sonnet-4-20250514"), tenant);

        let body = interpreter.build_request_body(&program, &[]);

        assert_eq!(body["model"], "claude-sonnet-4-20250514");
        assert_eq!(body["stream"], true);
        assert!(body["system"].is_string());
        assert!(body["messages"].is_array());
        assert!(body.get("thinking").is_none());
    }

    #[test]
    fn build_request_body_with_thinking() {
        use agent_fw_agent::{parse_conversation, ChatMessage, ModelId, ReasoningEffort};
        use agent_fw_core::tenant::TenantContext;

        let config = AnthropicConfig::new("test-key");
        let blueprint = AgentBlueprint::new(ModelId::new("claude-sonnet-4-20250514"), "prompt")
            .with_thinking_budget(10000)
            .with_reasoning_effort(ReasoningEffort::High);
        let interpreter = AnthropicInterpreter::new(config, blueprint);

        let conv = parse_conversation(vec![ChatMessage::user("Think carefully")]).unwrap();

        let tenant = TenantContext::new(agent_fw_core::id::TenantId::new("t1").unwrap());
        let program = ChatProgram::new(conv, ModelId::new(""), tenant);

        let body = interpreter.build_request_body(&program, &[]);

        assert!(body.get("thinking").is_some());
        assert_eq!(body["thinking"]["type"], "enabled");
        assert_eq!(body["thinking"]["budget_tokens"], 10000);
        assert!(body.get("output_config").is_none());
    }

    #[test]
    fn build_request_body_uses_adaptive_for_claude_4_6() {
        use agent_fw_agent::{parse_conversation, ChatMessage, ModelId, ReasoningEffort};
        use agent_fw_core::tenant::TenantContext;

        let config = AnthropicConfig::new("test-key");
        let blueprint = AgentBlueprint::new(ModelId::new("claude-opus-4-6"), "prompt")
            .with_reasoning_effort(ReasoningEffort::High);
        let interpreter = AnthropicInterpreter::new(config, blueprint);

        let conv = parse_conversation(vec![ChatMessage::user("Think carefully")]).unwrap();
        let tenant = TenantContext::new(agent_fw_core::id::TenantId::new("t1").unwrap());
        let program = ChatProgram::new(conv, ModelId::new(""), tenant);

        let body = interpreter.build_request_body(&program, &[]);

        assert_eq!(body["thinking"]["type"], "adaptive");
        assert_eq!(body["output_config"]["effort"], "high");
    }

    #[test]
    fn build_request_body_with_prompt_caching_marks_system_messages_and_tools() {
        use agent_fw_agent::{parse_conversation, ChatMessage, ModelId};
        use agent_fw_core::tenant::TenantContext;

        let config = AnthropicConfig::new("test-key");
        let blueprint =
            AgentBlueprint::new(ModelId::new("claude-opus-4-6"), "prompt").prompt_caching(true);
        let interpreter = AnthropicInterpreter::new(config, blueprint)
            .with_tool_dispatcher(Arc::new(TestDispatcher));

        let conv = parse_conversation(vec![
            ChatMessage::system("Be helpful"),
            ChatMessage::user("Hello"),
        ])
        .unwrap();
        let tenant = TenantContext::new(agent_fw_core::id::TenantId::new("t1").unwrap());
        let program = ChatProgram::new(conv, ModelId::new("claude-opus-4-6"), tenant);

        let extra_messages = vec![
            serde_json::json!({
                "role": "assistant",
                "content": [{
                    "type": "tool_use",
                    "id": "toolu_1",
                    "name": "lookup",
                    "input": {},
                }],
            }),
            serde_json::json!({
                "role": "user",
                "content": [{
                    "type": "tool_result",
                    "tool_use_id": "toolu_1",
                    "content": "{}",
                }],
            }),
        ];

        let body = interpreter.build_request_body(&program, &extra_messages);

        assert_eq!(
            body["system"][0]["cache_control"]["type"],
            serde_json::json!("ephemeral")
        );
        assert_eq!(
            body["messages"]
                .as_array()
                .and_then(|messages| messages.last())
                .and_then(|message| message.get("content"))
                .and_then(serde_json::Value::as_array)
                .and_then(|content| content.last())
                .and_then(|block| block.get("cache_control"))
                .and_then(|cache| cache.get("type")),
            Some(&serde_json::json!("ephemeral"))
        );
        assert_eq!(
            body["tools"][0]["cache_control"]["type"],
            serde_json::json!("ephemeral")
        );
    }
}
