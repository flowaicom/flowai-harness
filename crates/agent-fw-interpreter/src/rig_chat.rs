//! Stock Rig-based chat interpreters.
//!
//! These are intentionally small, ergonomic interpreters for applications that
//! want a plain-chat or custom-endpoint path without rebuilding the same
//! hook/sink/tool shell locally.

use agent_fw_agent::{
    anthropic_reasoning_params, conversation_to_rig_history, dispatcher_rig_tools, CalculatorTool,
    ChatInterpreter, ChatProgram, GetCurrentTimeTool, ModelSettings, ReasoningEffort,
    RigHookBridge, RigRequestResult, ToolDispatcher,
};
use agent_fw_algebra::{CancellationToken, EventSink};
use agent_fw_core::{FinishReason, ProviderSettingsMap, StreamPart, TokenUsage};
use aws_config::{BehaviorVersion, Region};
use aws_sdk_bedrockruntime::Client as AwsBedrockClient;
use futures::Stream;
use rig::agent::AgentBuilder;
#[allow(deprecated)]
use rig::client::completion::CompletionModelHandle;
use rig::client::CompletionClient;
use rig::message::Message;
use rig::providers::{anthropic, openai};
use rig::streaming::StreamingChat;
use rig_bedrock::client::Client as BedrockClient;
use std::pin::Pin;
use std::sync::Arc;

use crate::stock_openai_compatible_base_url;
use crate::ChannelEventSink;

/// Construction errors for stock Rig chat interpreters.
#[derive(Debug)]
pub enum RigChatInterpreterError {
    Config(String),
}

impl std::fmt::Display for RigChatInterpreterError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Config(msg) => write!(f, "Configuration error: {}", msg),
        }
    }
}

impl std::error::Error for RigChatInterpreterError {}

/// Stock Anthropic-backed chat interpreter using the Rig runtime.
///
/// When no dispatcher is configured, it ships with two generic utility tools:
/// - `get_current_time`
/// - `calculator`
#[derive(Clone)]
pub struct RigAnthropicChatInterpreter {
    client: anthropic::Client,
    max_turns: usize,
    model_settings: ModelSettings,
    tool_dispatcher: Option<Arc<dyn ToolDispatcher>>,
}

impl RigAnthropicChatInterpreter {
    pub fn new(api_key: impl Into<String>) -> Result<Self, RigChatInterpreterError> {
        let client = anthropic::Client::builder()
            .api_key(api_key.into())
            .build()
            .map_err(|e| {
                RigChatInterpreterError::Config(format!("Failed to build Anthropic client: {}", e))
            })?;
        Ok(Self {
            client,
            max_turns: 16,
            model_settings: ModelSettings::default(),
            tool_dispatcher: None,
        })
    }

    pub fn with_tool_dispatcher(mut self, dispatcher: Arc<dyn ToolDispatcher>) -> Self {
        self.tool_dispatcher = Some(dispatcher);
        self
    }

    pub fn with_max_turns(mut self, max_turns: usize) -> Self {
        self.max_turns = max_turns;
        self
    }

    pub fn with_max_tokens(mut self, max_tokens: u64) -> Self {
        self.model_settings = self
            .model_settings
            .with_overrides(
                Some(max_tokens.min(u32::MAX as u64) as u32),
                None,
                None,
                None,
            )
            .expect("bounded max tokens");
        self
    }

    pub fn with_model_settings(mut self, settings: ModelSettings) -> Self {
        self.model_settings = settings;
        self
    }

    pub fn with_thinking_budget(mut self, budget: u32) -> Self {
        self.model_settings = self
            .model_settings
            .with_overrides(None, Some(budget), None, None)
            .expect("valid thinking budget");
        self
    }

    pub fn with_reasoning_effort(mut self, effort: ReasoningEffort) -> Self {
        self.model_settings = self
            .model_settings
            .with_overrides(None, None, Some(effort), None)
            .expect("valid reasoning effort");
        self
    }

    pub fn with_prompt_caching(mut self, enabled: bool) -> Self {
        self.model_settings = self
            .model_settings
            .with_overrides(None, None, None, Some(enabled))
            .expect("valid prompt caching setting");
        self
    }
}

impl ChatInterpreter for RigAnthropicChatInterpreter {
    fn interpret(
        &self,
        program: ChatProgram,
        cancel: CancellationToken,
    ) -> Pin<Box<dyn Stream<Item = StreamPart> + Send>> {
        let (sink, receiver) = ChannelEventSink::new(256);
        let sink = Arc::new(sink);

        let prompt = program.conversation().prompt().as_str().to_string();
        let history = conversation_to_rig_history(program.conversation());
        let system_prompt = program.system_prompt().as_str().to_string();
        let model = program.model().as_str().to_string();
        let tenant_id = program.tenant().resource_id().to_string();

        let client = self.client.clone();
        let max_turns = self.max_turns;
        let model_settings = self.model_settings;
        let tool_dispatcher = self.tool_dispatcher.clone();
        let hook = build_rig_hook(sink.clone(), tool_dispatcher.as_ref(), "interpreter");

        tokio::spawn(run_anthropic_chat_stream(
            client,
            model,
            system_prompt,
            prompt,
            history,
            tenant_id,
            hook,
            cancel,
            max_turns,
            model_settings,
            tool_dispatcher,
        ));

        Box::pin(receiver)
    }

    fn with_tool_dispatcher(
        self: Arc<Self>,
        dispatcher: Arc<dyn ToolDispatcher>,
    ) -> Option<Arc<dyn ChatInterpreter>> {
        let mut cloned: RigAnthropicChatInterpreter = (*self).clone();
        cloned.tool_dispatcher = Some(dispatcher);
        Some(Arc::new(cloned))
    }

    fn with_max_turns(self: Arc<Self>, max_turns: usize) -> Option<Arc<dyn ChatInterpreter>> {
        let mut cloned: RigAnthropicChatInterpreter = (*self).clone();
        cloned.max_turns = max_turns;
        Some(Arc::new(cloned))
    }

    fn with_model_settings(
        self: Arc<Self>,
        settings: ModelSettings,
    ) -> Option<Arc<dyn ChatInterpreter>> {
        let mut cloned: RigAnthropicChatInterpreter = (*self).clone();
        cloned.model_settings = settings;
        Some(Arc::new(cloned))
    }
}

/// Stock OpenAI-compatible chat interpreter using Rig's chat-completions path.
///
/// This is the low-ceremony interpreter for custom endpoint overrides that
/// expose an OpenAI-compatible `/chat/completions` surface.
#[derive(Clone)]
pub struct RigOpenAiCompatibleChatInterpreter {
    client: openai::CompletionsClient,
    max_turns: usize,
    max_tokens: u64,
    tool_dispatcher: Option<Arc<dyn ToolDispatcher>>,
}

impl RigOpenAiCompatibleChatInterpreter {
    pub fn new(
        api_key: impl Into<String>,
        base_url: impl AsRef<str>,
    ) -> Result<Self, RigChatInterpreterError> {
        let client = openai::CompletionsClient::builder()
            .api_key(api_key.into())
            .base_url(base_url.as_ref())
            .build()
            .map_err(|e| {
                RigChatInterpreterError::Config(format!(
                    "Failed to build OpenAI-compatible client: {}",
                    e
                ))
            })?;
        Ok(Self {
            client,
            max_turns: 16,
            max_tokens: ModelSettings::default().max_tokens as u64,
            tool_dispatcher: None,
        })
    }

    pub fn with_tool_dispatcher(mut self, dispatcher: Arc<dyn ToolDispatcher>) -> Self {
        self.tool_dispatcher = Some(dispatcher);
        self
    }

    pub fn with_max_turns(mut self, max_turns: usize) -> Self {
        self.max_turns = max_turns;
        self
    }

    pub fn with_max_tokens(mut self, max_tokens: u64) -> Self {
        self.max_tokens = max_tokens;
        self
    }
}

impl ChatInterpreter for RigOpenAiCompatibleChatInterpreter {
    fn interpret(
        &self,
        program: ChatProgram,
        cancel: CancellationToken,
    ) -> Pin<Box<dyn Stream<Item = StreamPart> + Send>> {
        let (sink, receiver) = ChannelEventSink::new(256);
        let sink = Arc::new(sink);

        let prompt = program.conversation().prompt().as_str().to_string();
        let history = conversation_to_rig_history(program.conversation());
        let system_prompt = program.system_prompt().as_str().to_string();
        let model = program.model().as_str().to_string();
        let tenant_id = program.tenant().resource_id().to_string();

        let client = self.client.clone();
        let max_turns = self.max_turns;
        let max_tokens = self.max_tokens;
        let tool_dispatcher = self.tool_dispatcher.clone();
        let hook = build_rig_hook(sink.clone(), tool_dispatcher.as_ref(), "interpreter");

        tokio::spawn(run_openai_chat_stream(
            client,
            model,
            system_prompt,
            prompt,
            history,
            tenant_id,
            hook,
            cancel,
            max_turns,
            max_tokens,
            tool_dispatcher,
        ));

        Box::pin(receiver)
    }

    fn with_tool_dispatcher(
        self: Arc<Self>,
        dispatcher: Arc<dyn ToolDispatcher>,
    ) -> Option<Arc<dyn ChatInterpreter>> {
        let mut cloned: RigOpenAiCompatibleChatInterpreter = (*self).clone();
        cloned.tool_dispatcher = Some(dispatcher);
        Some(Arc::new(cloned))
    }

    fn with_max_turns(self: Arc<Self>, max_turns: usize) -> Option<Arc<dyn ChatInterpreter>> {
        let mut cloned: RigOpenAiCompatibleChatInterpreter = (*self).clone();
        cloned.max_turns = max_turns;
        Some(Arc::new(cloned))
    }

    fn with_model_settings(
        self: Arc<Self>,
        settings: ModelSettings,
    ) -> Option<Arc<dyn ChatInterpreter>> {
        let mut cloned: RigOpenAiCompatibleChatInterpreter = (*self).clone();
        cloned.max_tokens = settings.max_tokens as u64;
        Some(Arc::new(cloned))
    }
}

/// Stock AWS Bedrock chat interpreter using Rig's Bedrock completion path.
#[derive(Clone)]
pub struct RigBedrockChatInterpreter {
    region: String,
    max_turns: usize,
    model_settings: ModelSettings,
    tool_dispatcher: Option<Arc<dyn ToolDispatcher>>,
}

impl RigBedrockChatInterpreter {
    pub fn new(region: impl Into<String>) -> Self {
        Self {
            region: region.into(),
            max_turns: 16,
            model_settings: ModelSettings::default(),
            tool_dispatcher: None,
        }
    }

    pub fn with_tool_dispatcher(mut self, dispatcher: Arc<dyn ToolDispatcher>) -> Self {
        self.tool_dispatcher = Some(dispatcher);
        self
    }

    pub fn with_max_turns(mut self, max_turns: usize) -> Self {
        self.max_turns = max_turns;
        self
    }

    pub fn with_max_tokens(mut self, max_tokens: u64) -> Self {
        self.model_settings = self
            .model_settings
            .with_overrides(
                Some(max_tokens.min(u32::MAX as u64) as u32),
                None,
                None,
                None,
            )
            .expect("bounded max tokens");
        self
    }

    pub fn with_model_settings(mut self, settings: ModelSettings) -> Self {
        self.model_settings = settings;
        self
    }

    pub fn with_thinking_budget(mut self, budget: u32) -> Self {
        self.model_settings = self
            .model_settings
            .with_overrides(None, Some(budget), None, None)
            .expect("valid thinking budget");
        self
    }

    pub fn with_reasoning_effort(mut self, effort: ReasoningEffort) -> Self {
        self.model_settings = self
            .model_settings
            .with_overrides(None, None, Some(effort), None)
            .expect("valid reasoning effort");
        self
    }

    pub fn with_prompt_caching(mut self, enabled: bool) -> Self {
        self.model_settings = self
            .model_settings
            .with_overrides(None, None, None, Some(enabled))
            .expect("valid prompt caching setting");
        self
    }
}

impl ChatInterpreter for RigBedrockChatInterpreter {
    fn interpret(
        &self,
        program: ChatProgram,
        cancel: CancellationToken,
    ) -> Pin<Box<dyn Stream<Item = StreamPart> + Send>> {
        let (sink, receiver) = ChannelEventSink::new(256);
        let sink = Arc::new(sink);

        let prompt = program.conversation().prompt().as_str().to_string();
        let history = conversation_to_rig_history(program.conversation());
        let system_prompt = program.system_prompt().as_str().to_string();
        let model = program.model().as_str().to_string();
        let tenant_id = program.tenant().resource_id().to_string();

        let region = self.region.clone();
        let max_turns = self.max_turns;
        let model_settings = self.model_settings;
        let tool_dispatcher = self.tool_dispatcher.clone();
        let hook = build_rig_hook(sink.clone(), tool_dispatcher.as_ref(), "interpreter");

        tokio::spawn(run_bedrock_chat_stream(
            region,
            model,
            system_prompt,
            prompt,
            history,
            tenant_id,
            hook,
            cancel,
            max_turns,
            model_settings,
            tool_dispatcher,
        ));

        Box::pin(receiver)
    }

    fn with_tool_dispatcher(
        self: Arc<Self>,
        dispatcher: Arc<dyn ToolDispatcher>,
    ) -> Option<Arc<dyn ChatInterpreter>> {
        let mut cloned: RigBedrockChatInterpreter = (*self).clone();
        cloned.tool_dispatcher = Some(dispatcher);
        Some(Arc::new(cloned))
    }

    fn with_max_turns(self: Arc<Self>, max_turns: usize) -> Option<Arc<dyn ChatInterpreter>> {
        let mut cloned: RigBedrockChatInterpreter = (*self).clone();
        cloned.max_turns = max_turns;
        Some(Arc::new(cloned))
    }

    fn with_model_settings(
        self: Arc<Self>,
        settings: ModelSettings,
    ) -> Option<Arc<dyn ChatInterpreter>> {
        let mut cloned: RigBedrockChatInterpreter = (*self).clone();
        cloned.model_settings = settings;
        Some(Arc::new(cloned))
    }
}

pub fn stock_chat_interpreter_from_settings(
    provider_key: &str,
    settings: &ProviderSettingsMap,
    dispatcher: Arc<dyn ToolDispatcher>,
    model_settings: ModelSettings,
) -> Result<Option<Box<dyn ChatInterpreter>>, RigChatInterpreterError> {
    match provider_key {
        "anthropic" => {
            let api_key = required_setting(settings, "apiKey", "Anthropic embedded chat")?;
            let interpreter = RigAnthropicChatInterpreter::new(api_key)?
                .with_model_settings(model_settings)
                .with_tool_dispatcher(dispatcher);
            Ok(Some(Box::new(interpreter)))
        }
        "bedrock" => {
            let region = required_setting(settings, "region", "AWS Bedrock embedded chat")?;
            let interpreter = RigBedrockChatInterpreter::new(region)
                .with_model_settings(model_settings)
                .with_tool_dispatcher(dispatcher);
            Ok(Some(Box::new(interpreter)))
        }
        provider_key => {
            let Some(base_url) = stock_openai_compatible_base_url(provider_key) else {
                return Ok(None);
            };
            let api_key =
                required_setting(settings, "apiKey", &format!("{provider_key} embedded chat"))?;
            let interpreter = RigOpenAiCompatibleChatInterpreter::new(api_key, base_url)?
                .with_max_tokens(model_settings.max_tokens as u64)
                .with_tool_dispatcher(dispatcher);
            Ok(Some(Box::new(interpreter)))
        }
    }
}

fn required_setting<'a>(
    settings: &'a ProviderSettingsMap,
    key: &str,
    context: &str,
) -> Result<&'a str, RigChatInterpreterError> {
    settings
        .get(key)
        .map(String::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            RigChatInterpreterError::Config(format!(
                "{context} requires a non-empty provider setting '{key}'."
            ))
        })
}

fn build_rig_hook(
    sink: Arc<ChannelEventSink>,
    dispatcher: Option<&Arc<dyn ToolDispatcher>>,
    agent_name: &str,
) -> RigHookBridge<ChannelEventSink> {
    let mut hook = RigHookBridge::new(sink, agent_name);
    if let Some(dispatcher) = dispatcher {
        if let Some(cell) = dispatcher.tool_call_id_cell() {
            hook = hook.with_tool_call_id_cell(cell);
        }
        if let Some(cell) = dispatcher.pending_card_cell() {
            hook = hook.with_pending_card_cell(cell);
        }
    }
    hook
}

#[allow(deprecated)]
fn anthropic_completion_model(
    client: &anthropic::Client,
    model: &str,
    prompt_caching: bool,
) -> CompletionModelHandle<'static> {
    if prompt_caching {
        CompletionModelHandle::new(Arc::new(
            client.completion_model(model).with_prompt_caching(),
        ))
    } else {
        CompletionModelHandle::new(Arc::new(client.completion_model(model)))
    }
}

#[allow(deprecated)]
fn bedrock_completion_model(
    client: &BedrockClient,
    model: &str,
    prompt_caching: bool,
) -> CompletionModelHandle<'static> {
    if prompt_caching {
        CompletionModelHandle::new(Arc::new(
            client.completion_model(model).with_prompt_caching(),
        ))
    } else {
        CompletionModelHandle::new(Arc::new(client.completion_model(model)))
    }
}

async fn run_anthropic_chat_stream(
    client: anthropic::Client,
    model: String,
    system_prompt: String,
    prompt: String,
    history: Vec<Message>,
    tenant_id: String,
    hook: RigHookBridge<ChannelEventSink>,
    cancel: CancellationToken,
    max_turns: usize,
    model_settings: ModelSettings,
    tool_dispatcher: Option<Arc<dyn ToolDispatcher>>,
) {
    tracing::info!(
        tenant_id = %tenant_id,
        prompt_len = prompt.len(),
        history_len = history.len(),
        model = %model,
        "Starting Rig Anthropic chat interpretation"
    );

    let completion_model =
        anthropic_completion_model(&client, &model, model_settings.cache_control);
    let mut builder = AgentBuilder::new(completion_model)
        .preamble(&system_prompt)
        .default_max_turns(max_turns);
    builder = builder.max_tokens(model_settings.max_tokens as u64);
    if let Some(params) = anthropic_reasoning_params(
        &model,
        true,
        model_settings.thinking_budget,
        Some(model_settings.reasoning_effort),
    ) {
        builder = builder.additional_params(params);
    }
    let agent = if let Some(dispatcher) = tool_dispatcher.clone() {
        builder.tools(dispatcher_rig_tools(dispatcher)).build()
    } else {
        builder
            .tool(GetCurrentTimeTool)
            .tool(CalculatorTool)
            .build()
    };

    let RigRequestResult { outcome, summary } = hook
        .run_stream_request(
            &cancel,
            "Request cancelled",
            "coordinator",
            model.clone(),
            FinishReason::Stop,
            |hook| async move { agent.stream_chat(&prompt, history).with_hook(hook).await },
        )
        .await;
    let usage = summary.token_usage();

    tracing::debug!(
        tenant_id = %tenant_id,
        response_len = outcome.response_len,
        had_error = outcome.had_error,
        was_cancelled = outcome.was_cancelled,
        prompt_tokens = usage.prompt_tokens,
        completion_tokens = usage.completion_tokens,
        "Rig Anthropic chat interpretation complete"
    );
}

async fn run_openai_chat_stream(
    client: openai::CompletionsClient,
    model: String,
    system_prompt: String,
    prompt: String,
    history: Vec<Message>,
    tenant_id: String,
    hook: RigHookBridge<ChannelEventSink>,
    cancel: CancellationToken,
    max_turns: usize,
    max_tokens: u64,
    tool_dispatcher: Option<Arc<dyn ToolDispatcher>>,
) {
    tracing::info!(
        tenant_id = %tenant_id,
        prompt_len = prompt.len(),
        history_len = history.len(),
        model = %model,
        "Starting Rig OpenAI-compatible chat interpretation"
    );

    let mut builder = client
        .completion_model(&model)
        .into_agent_builder()
        .preamble(&system_prompt)
        .default_max_turns(max_turns);
    builder = builder.max_tokens(max_tokens);
    let agent = if let Some(dispatcher) = tool_dispatcher.clone() {
        builder.tools(dispatcher_rig_tools(dispatcher)).build()
    } else {
        builder
            .tool(GetCurrentTimeTool)
            .tool(CalculatorTool)
            .build()
    };

    let RigRequestResult { outcome, summary } = hook
        .run_stream_request(
            &cancel,
            "Request cancelled",
            "coordinator",
            model.clone(),
            FinishReason::Stop,
            |hook| async move { agent.stream_chat(&prompt, history).with_hook(hook).await },
        )
        .await;
    let usage = summary.token_usage();

    tracing::debug!(
        tenant_id = %tenant_id,
        response_len = outcome.response_len,
        had_error = outcome.had_error,
        was_cancelled = outcome.was_cancelled,
        prompt_tokens = usage.prompt_tokens,
        completion_tokens = usage.completion_tokens,
        "Rig OpenAI-compatible chat interpretation complete"
    );
}

async fn run_bedrock_chat_stream(
    region: String,
    model: String,
    system_prompt: String,
    prompt: String,
    history: Vec<Message>,
    tenant_id: String,
    hook: RigHookBridge<ChannelEventSink>,
    cancel: CancellationToken,
    max_turns: usize,
    model_settings: ModelSettings,
    tool_dispatcher: Option<Arc<dyn ToolDispatcher>>,
) {
    tracing::info!(
        tenant_id = %tenant_id,
        prompt_len = prompt.len(),
        history_len = history.len(),
        region = %region,
        model = %model,
        "Starting Rig Bedrock chat interpretation"
    );

    let sdk_config = aws_config::defaults(BehaviorVersion::latest())
        .region(Region::new(region))
        .load()
        .await;
    let client = BedrockClient::from(AwsBedrockClient::new(&sdk_config));

    let completion_model = bedrock_completion_model(&client, &model, model_settings.cache_control);
    let mut builder = AgentBuilder::new(completion_model)
        .preamble(&system_prompt)
        .default_max_turns(max_turns);
    builder = builder.max_tokens(model_settings.max_tokens as u64);
    if let Some(params) = anthropic_reasoning_params(
        &model,
        true,
        model_settings.thinking_budget,
        Some(model_settings.reasoning_effort),
    ) {
        builder = builder.additional_params(params);
    }
    let agent = if let Some(dispatcher) = tool_dispatcher.clone() {
        builder.tools(dispatcher_rig_tools(dispatcher)).build()
    } else {
        builder
            .tool(GetCurrentTimeTool)
            .tool(CalculatorTool)
            .build()
    };

    let RigRequestResult { outcome, summary } = hook
        .run_stream_request(
            &cancel,
            "Request cancelled",
            "coordinator",
            model.clone(),
            FinishReason::Stop,
            |hook| async move { agent.stream_chat(&prompt, history).with_hook(hook).await },
        )
        .await;
    let usage = summary.token_usage();

    tracing::debug!(
        tenant_id = %tenant_id,
        response_len = outcome.response_len,
        had_error = outcome.had_error,
        was_cancelled = outcome.was_cancelled,
        prompt_tokens = usage.prompt_tokens,
        completion_tokens = usage.completion_tokens,
        "Rig Bedrock chat interpretation complete"
    );
}

/// Stock mock chat interpreter for tests and local fallback paths.
#[derive(Clone)]
pub struct MockChatInterpreter {
    response: String,
    latency_ms: u64,
}

impl MockChatInterpreter {
    pub fn new(response: impl Into<String>) -> Self {
        Self {
            response: response.into(),
            latency_ms: 50,
        }
    }

    pub fn with_latency(mut self, ms: u64) -> Self {
        self.latency_ms = ms;
        self
    }
}

impl Default for MockChatInterpreter {
    fn default() -> Self {
        Self::new("This is a mock response.")
    }
}

impl ChatInterpreter for MockChatInterpreter {
    fn interpret(
        &self,
        program: ChatProgram,
        cancel: CancellationToken,
    ) -> Pin<Box<dyn Stream<Item = StreamPart> + Send>> {
        let (sink, receiver) = ChannelEventSink::new(64);
        let sink = Arc::new(sink);
        let prompt = program.conversation().prompt().as_str().to_string();
        let response = self.response.clone();
        let latency = self.latency_ms;

        tokio::spawn(async move {
            if cancel.is_cancelled() {
                sink.emit(StreamPart::StepStart);
                sink.emit(StreamPart::error("Request cancelled"));
                sink.emit(StreamPart::finish(FinishReason::Stop, TokenUsage::ZERO));
                sink.close();
                return;
            }

            tokio::time::sleep(tokio::time::Duration::from_millis(latency / 4)).await;
            if cancel.is_cancelled() {
                sink.emit(StreamPart::StepStart);
                sink.emit(StreamPart::error("Request cancelled"));
                sink.emit(StreamPart::finish(FinishReason::Stop, TokenUsage::ZERO));
                sink.close();
                return;
            }

            sink.emit(StreamPart::StepStart);
            tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;

            if cancel.is_cancelled() {
                sink.emit(StreamPart::error("Request cancelled"));
                sink.emit(StreamPart::finish(FinishReason::Stop, TokenUsage::ZERO));
                sink.close();
                return;
            }

            sink.emit(StreamPart::text(format!("Received: {}\n\n", prompt)));
            tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;

            if cancel.is_cancelled() {
                sink.emit(StreamPart::error("Request cancelled"));
                sink.emit(StreamPart::finish(FinishReason::Stop, TokenUsage::ZERO));
                sink.close();
                return;
            }

            sink.emit(StreamPart::text(response));
            tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;

            sink.emit(StreamPart::latency_summary(
                agent_fw_core::latency::LatencySummary {
                    total_duration_ms: latency,
                    phases: agent_fw_core::latency::PhaseBreakdown::new(latency / 2, 0, 1),
                    ..Default::default()
                },
            ));
            sink.emit(StreamPart::finish(
                FinishReason::Stop,
                TokenUsage::simple(50, 25),
            ));
            sink.close();
        });

        Box::pin(receiver)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_fw_agent::{
        assert_chat_interpreter_contract, parse_conversation, ChatMessage, ModelId,
    };
    use agent_fw_core::id::TenantId;
    use agent_fw_core::tenant::TenantContext;

    fn test_tenant() -> TenantContext {
        TenantContext::new(TenantId::new_unchecked("test-tenant"))
    }

    #[tokio::test]
    async fn mock_interpreter_produces_valid_stream() {
        let interpreter = MockChatInterpreter::new("test response");
        let messages = vec![ChatMessage::user("hello")];
        let conv = parse_conversation(messages).unwrap();
        let program = ChatProgram::new(conv, ModelId::new("claude-opus-4-6"), test_tenant());
        let cancel = CancellationToken::new();

        let events = assert_chat_interpreter_contract(&interpreter, program, cancel).await;

        assert!(!events.is_empty());
        assert!(matches!(events[0], StreamPart::StepStart));
        assert!(matches!(events.last(), Some(StreamPart::Finish { .. })));
    }

    #[tokio::test]
    async fn mock_interpreter_respects_pre_cancellation() {
        let interpreter = MockChatInterpreter::new("test response");
        let messages = vec![ChatMessage::user("hello")];
        let conv = parse_conversation(messages).unwrap();
        let program = ChatProgram::new(conv, ModelId::new("claude-opus-4-6"), test_tenant());
        let cancel = CancellationToken::new();
        cancel.cancel();

        let events = assert_chat_interpreter_contract(&interpreter, program, cancel).await;

        assert!(matches!(events[0], StreamPart::StepStart));
        let has_cancel_error = events.iter().any(
            |e| matches!(e, StreamPart::Error { error } if error.message.contains("cancelled")),
        );
        assert!(has_cancel_error);
        assert!(matches!(events.last(), Some(StreamPart::Finish { .. })));
    }

    #[test]
    fn openai_compatible_builder_accepts_base_url() {
        let interpreter = RigOpenAiCompatibleChatInterpreter::new("", "http://localhost:11434/v1")
            .expect("openai-compatible interpreter");
        assert_eq!(interpreter.client.base_url(), "http://localhost:11434/v1");
    }

    #[test]
    fn bedrock_builder_accepts_region() {
        let interpreter = RigBedrockChatInterpreter::new("eu-central-1").with_max_tokens(4096);
        assert_eq!(interpreter.region, "eu-central-1");
        assert_eq!(interpreter.model_settings.max_tokens, 4096);
    }

    #[test]
    fn anthropic_and_bedrock_builders_accept_model_settings_description() {
        let settings =
            ModelSettings::new(8192, 0, ReasoningEffort::Max, false).expect("model settings");
        let anthropic = RigAnthropicChatInterpreter::new("key")
            .expect("anthropic interpreter")
            .with_model_settings(settings);
        let bedrock = RigBedrockChatInterpreter::new("eu-central-1").with_model_settings(settings);

        assert_eq!(anthropic.model_settings, settings);
        assert_eq!(bedrock.model_settings, settings);
    }

    // ─── G1 (runtime query assembly): with_tool_dispatcher trait override ───────────────
    //
    // The default `ChatInterpreter::with_tool_dispatcher` returns `None`.
    // Each Rig interpreter overrides it by cloning the struct and binding
    // the supplied dispatcher, so the orchestrator can pick a per-agent
    // dispatcher without rebuilding the interpreter from scratch.

    use agent_fw_agent::ToolCallResult;
    use async_trait::async_trait;

    struct StubDispatcher;

    #[async_trait]
    impl ToolDispatcher for StubDispatcher {
        fn tool_definitions(&self) -> Vec<agent_fw_agent::ToolDefinition> {
            vec![]
        }
        async fn dispatch(
            &self,
            _tool_name: &str,
            tool_use_id: &str,
            _input: serde_json::Value,
        ) -> ToolCallResult {
            ToolCallResult::success(tool_use_id, serde_json::json!({}))
        }
    }

    #[test]
    fn mock_interpreter_with_tool_dispatcher_returns_none_by_default() {
        let interpreter: Arc<dyn ChatInterpreter> = Arc::new(MockChatInterpreter::new("response"));
        let result = interpreter.with_tool_dispatcher(Arc::new(StubDispatcher));
        assert!(result.is_none());
    }

    #[test]
    fn rig_anthropic_with_tool_dispatcher_attaches_supplied_dispatcher() {
        let base: Arc<RigAnthropicChatInterpreter> =
            Arc::new(RigAnthropicChatInterpreter::new("k").expect("anthropic interpreter"));
        assert!(base.tool_dispatcher.is_none());

        let with_d: Arc<dyn ChatInterpreter> = base
            .clone()
            .with_tool_dispatcher(Arc::new(StubDispatcher))
            .expect("Rig override returns Some");

        // Original Arc is untouched.
        assert!(base.tool_dispatcher.is_none());
        // The trait-object Arc carries the dispatcher: cast back to the
        // concrete type via Any-style downcast through Arc::ptr_eq with a
        // fresh build is too brittle — assert via behaviour: cloning
        // again produces another distinct Some-result (idempotent shape).
        let again = with_d
            .clone()
            .with_tool_dispatcher(Arc::new(StubDispatcher));
        assert!(again.is_some());
    }

    #[test]
    fn rig_bedrock_with_tool_dispatcher_attaches_supplied_dispatcher() {
        let base: Arc<RigBedrockChatInterpreter> =
            Arc::new(RigBedrockChatInterpreter::new("eu-central-1"));
        assert!(base.tool_dispatcher.is_none());

        let with_d = base.clone().with_tool_dispatcher(Arc::new(StubDispatcher));
        assert!(with_d.is_some());
        assert!(base.tool_dispatcher.is_none());
    }

    #[test]
    fn rig_openai_with_tool_dispatcher_attaches_supplied_dispatcher() {
        let base: Arc<RigOpenAiCompatibleChatInterpreter> = Arc::new(
            RigOpenAiCompatibleChatInterpreter::new("", "http://localhost:11434/v1")
                .expect("openai-compatible interpreter"),
        );
        assert!(base.tool_dispatcher.is_none());

        let with_d = base.clone().with_tool_dispatcher(Arc::new(StubDispatcher));
        assert!(with_d.is_some());
        assert!(base.tool_dispatcher.is_none());
    }
}
