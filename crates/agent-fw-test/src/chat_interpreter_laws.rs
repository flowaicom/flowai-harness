//! ChatInterpreter algebraic law test harnesses.
//!
//! # Laws
//!
//! - L1. Termination: Every stream emits `StreamPart::Finish { .. }` and completes.
//! - L2. Ordering: `StepStart` precedes all other events in each turn.
//! - L3. Cancellation: After the token is cancelled, the stream completes.
//!
//! # Usage
//!
//! ```ignore
//! #[tokio::test]
//! async fn my_interpreter_satisfies_laws() {
//!     let interp = MyInterpreter::new();
//!     agent_fw_test::chat_interpreter_laws::test_all(&interp).await;
//! }
//! ```

use agent_fw_agent::{parse_conversation, ChatInterpreter, ChatMessage, ChatProgram, ModelId};
use agent_fw_algebra::CancellationToken;
use agent_fw_core::id::TenantId;
use agent_fw_core::stream_part::FinishReason;
use agent_fw_core::tenant::TenantContext;
use agent_fw_core::usage::TokenUsage;
use agent_fw_core::StreamPart;
use futures::StreamExt;
use std::pin::Pin;

// ─── Test ChatInterpreter ────────────────────────────────────────────

/// Minimal interpreter that emits StepStart → Text → Finish.
struct LawInterpreter;

impl ChatInterpreter for LawInterpreter {
    fn interpret(
        &self,
        _program: ChatProgram,
        _cancel: CancellationToken,
    ) -> Pin<Box<dyn futures::Stream<Item = StreamPart> + Send>> {
        Box::pin(futures::stream::iter(vec![
            StreamPart::StepStart,
            StreamPart::text("hello"),
            StreamPart::finish(FinishReason::Stop, TokenUsage::ZERO),
        ]))
    }
}

/// Interpreter that respects pre-cancellation.
/// When the token is already cancelled, emits only StepStart + Finish.
struct CancellableInterpreter;

impl ChatInterpreter for CancellableInterpreter {
    fn interpret(
        &self,
        _program: ChatProgram,
        cancel: CancellationToken,
    ) -> Pin<Box<dyn futures::Stream<Item = StreamPart> + Send>> {
        if cancel.is_cancelled() {
            Box::pin(futures::stream::iter(vec![
                StreamPart::StepStart,
                StreamPart::finish(FinishReason::Stop, TokenUsage::ZERO),
            ]))
        } else {
            Box::pin(futures::stream::iter(vec![
                StreamPart::StepStart,
                StreamPart::text("hello"),
                StreamPart::finish(FinishReason::Stop, TokenUsage::ZERO),
            ]))
        }
    }
}

// ─── Helpers ─────────────────────────────────────────────────────────

fn test_program() -> ChatProgram {
    let msgs = vec![ChatMessage::user("test")];
    let conv = parse_conversation(msgs).expect("valid conversation");
    let tenant = TenantContext::new(TenantId::new_unchecked("law-test"));
    ChatProgram::new(conv, ModelId::new("test-model"), tenant)
}

// ─── Public Law Test Functions ───────────────────────────────────────

/// Run all ChatInterpreter laws.
pub async fn test_all(interpreter: &dyn ChatInterpreter) {
    law_termination(interpreter).await;
    law_ordering(interpreter).await;
}

/// Run all laws including cancellation (requires CancellableInterpreter-like behavior).
pub async fn test_all_with_cancellation(interpreter: &dyn ChatInterpreter) {
    law_termination(interpreter).await;
    law_ordering(interpreter).await;
    law_cancellation(interpreter).await;
}

/// L1 (Termination): Stream emits Finish and completes.
pub async fn law_termination(interpreter: &dyn ChatInterpreter) {
    let program = test_program();
    let cancel = CancellationToken::new();
    let mut stream = interpreter.interpret(program, cancel);

    let mut saw_finish = false;
    while let Some(part) = stream.next().await {
        if matches!(part, StreamPart::Finish { .. }) {
            saw_finish = true;
        }
    }

    assert!(saw_finish, "L1: stream must emit StreamPart::Finish");
}

/// L2 (Ordering): StepStart precedes all other events.
pub async fn law_ordering(interpreter: &dyn ChatInterpreter) {
    let program = test_program();
    let cancel = CancellationToken::new();
    let mut stream = interpreter.interpret(program, cancel);

    let mut events = Vec::new();
    while let Some(part) = stream.next().await {
        events.push(part);
    }

    assert!(
        !events.is_empty(),
        "L2: stream must produce at least one event"
    );

    // First non-finish event must be StepStart
    let first = &events[0];
    assert!(
        matches!(first, StreamPart::StepStart),
        "L2: first event must be StepStart"
    );

    // StepStart must come before any Text or ToolInvocation
    let step_start_idx = events
        .iter()
        .position(|e| matches!(e, StreamPart::StepStart));
    let first_content_idx = events
        .iter()
        .position(|e| matches!(e, StreamPart::Text { .. } | StreamPart::ToolInvocation(_)));

    if let (Some(ss), Some(fc)) = (step_start_idx, first_content_idx) {
        assert!(
            ss < fc,
            "L2: StepStart (index {ss}) must precede content events (index {fc})"
        );
    }
}

/// L3 (Cancellation): After cancellation, stream completes promptly.
pub async fn law_cancellation(interpreter: &dyn ChatInterpreter) {
    let program = test_program();
    let cancel = CancellationToken::new();
    cancel.cancel();

    let mut stream = interpreter.interpret(program, cancel);

    let mut event_count = 0;
    while let Some(_part) = stream.next().await {
        event_count += 1;
        // Safety: prevent infinite loops in buggy interpreters
        if event_count > 100 {
            panic!("L3: stream did not terminate after cancellation (>100 events)");
        }
    }

    // Stream completed — that's the main assertion.
    // A well-behaved interpreter should emit few events (StepStart + Finish at most).
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn law_interpreter_satisfies_all_laws() {
        test_all(&LawInterpreter).await;
    }

    #[tokio::test]
    async fn cancellable_interpreter_satisfies_all_laws() {
        test_all_with_cancellation(&CancellableInterpreter).await;
    }

    #[tokio::test]
    async fn cancellation_terminates_stream() {
        law_cancellation(&CancellableInterpreter).await;
    }
}
