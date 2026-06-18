//! ChatInterpreter trait — the effectful interpreter for ChatPrograms.
//!
//! # Algebraic Laws
//!
//! 1. **Termination**: Every stream produced by `interpret` must eventually
//!    emit a `StreamPart::Finish` event and then complete. This ensures
//!    consumers can rely on bounded execution.
//!
//! 2. **Ordering**: Within each turn, a `StepStart` event precedes all
//!    other events (text, tool calls, etc.). This provides a well-defined
//!    framing for streaming consumers.
//!
//! 3. **Idempotence (structural)**: Given the same `ChatProgram` and
//!    deterministic external conditions, the interpreter produces a
//!    structurally equivalent stream of events. In practice, LLM outputs
//!    are non-deterministic, but the event *structure* (ordering, types,
//!    finish semantics) is deterministic.
//!
//! These laws support the programs-as-values discipline:
//! - **L1 Purity** of `ChatProgram` means the interpreter is the sole
//!   source of effects.
//! - **L2 Referential Transparency** means the same program always
//!   triggers the same interpreter behavior.
//! - **L3 Composition Associativity** is preserved because the
//!   interpreter treats each program independently.

use agent_fw_algebra::CancellationToken;
use agent_fw_core::StreamPart;
use async_trait::async_trait;
use futures::Stream;
use std::pin::Pin;
use std::sync::Arc;

use crate::conversation::ChatProgram;
use crate::model::ModelSettings;
use crate::tool_dispatch::ToolDispatcher;

/// Trait for interpreting ChatPrograms into event streams.
///
/// This is the effectful counterpart to the pure `ChatProgram` value.
/// Implementations bridge to specific LLM providers (Anthropic, OpenAI, etc.).
#[async_trait]
pub trait ChatInterpreter: Send + Sync {
    /// Interpret a ChatProgram, producing a stream of events.
    ///
    /// The stream must eventually emit a `StreamPart::Finish` event.
    /// The cancel token should be checked periodically for cooperative termination.
    fn interpret(
        &self,
        program: ChatProgram,
        cancel: CancellationToken,
    ) -> Pin<Box<dyn Stream<Item = StreamPart> + Send>>;

    /// Return a clone of this interpreter with the given tool dispatcher
    /// attached, used by orchestrators that bind a different dispatcher
    /// per sub-agent invocation.
    ///
    /// Returns `None` when the implementation does not carry an internal
    /// dispatcher (the default). Implementations that do (the Rig
    /// interpreters) override to return `Some(arc)` whose subsequent
    /// `interpret(...)` call uses the supplied dispatcher.
    ///
    /// Mirrors [`ToolDispatcher::with_event_sink`](crate::tool_dispatch::ToolDispatcher::with_event_sink).
    fn with_tool_dispatcher(
        self: Arc<Self>,
        _dispatcher: Arc<dyn ToolDispatcher>,
    ) -> Option<Arc<dyn ChatInterpreter>> {
        None
    }

    /// Return a clone of this interpreter with a max-turn override attached.
    ///
    /// Interpreters that support bounded multi-turn tool loops override this.
    /// The default keeps older implementations source-compatible and signals to
    /// callers that this interpreter cannot apply the override itself.
    fn with_max_turns(self: Arc<Self>, _max_turns: usize) -> Option<Arc<dyn ChatInterpreter>> {
        None
    }

    /// Return a clone of this interpreter with provider-neutral model settings.
    ///
    /// Runtime-owned callers use this to make special-purpose invocations
    /// explicit, such as strict JSON judge calls that should use a tighter
    /// token cap and lower effort than normal agents.
    fn with_model_settings(
        self: Arc<Self>,
        _settings: ModelSettings,
    ) -> Option<Arc<dyn ChatInterpreter>> {
        None
    }
}
