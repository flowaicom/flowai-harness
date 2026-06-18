//! Reusable ChatInterpreter contract tests.
//!
//! These helpers let concrete interpreters prove the same structural laws:
//! a turn starts with `StepStart`, finishes exactly once with `Finish`,
//! and does not emit terminal events out of order.

use agent_fw_algebra::CancellationToken;
use agent_fw_core::StreamPart;
use futures::StreamExt;

use crate::{ChatInterpreter, ChatProgram};

/// Assert the structural stream laws for an interpreted turn.
pub fn assert_chat_interpreter_events(events: &[StreamPart]) {
    assert!(
        !events.is_empty(),
        "ChatInterpreter stream must not be empty"
    );
    assert!(
        matches!(events.first(), Some(StreamPart::StepStart)),
        "ChatInterpreter stream must begin with StepStart"
    );

    let finish_positions: Vec<_> = events
        .iter()
        .enumerate()
        .filter_map(|(idx, event)| matches!(event, StreamPart::Finish { .. }).then_some(idx))
        .collect();

    assert_eq!(
        finish_positions.len(),
        1,
        "ChatInterpreter stream must emit exactly one Finish event"
    );
    assert_eq!(
        finish_positions[0],
        events.len() - 1,
        "Finish must be the terminal event in the stream"
    );
}

/// Execute an interpreter and assert the structural stream laws.
pub async fn assert_chat_interpreter_contract<I: ChatInterpreter + ?Sized>(
    interpreter: &I,
    program: ChatProgram,
    cancel: CancellationToken,
) -> Vec<StreamPart> {
    let events: Vec<_> = interpreter.interpret(program, cancel).collect().await;
    assert_chat_interpreter_events(&events);
    events
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_fw_core::{FinishReason, TokenUsage};

    #[test]
    fn accepts_well_formed_turn() {
        assert_chat_interpreter_events(&[
            StreamPart::StepStart,
            StreamPart::text("hello"),
            StreamPart::finish(FinishReason::Stop, TokenUsage::simple(1, 1)),
        ]);
    }

    #[test]
    #[should_panic(expected = "StepStart")]
    fn rejects_missing_step_start() {
        assert_chat_interpreter_events(&[
            StreamPart::text("hello"),
            StreamPart::finish(FinishReason::Stop, TokenUsage::simple(1, 1)),
        ]);
    }

    #[test]
    #[should_panic(expected = "exactly one Finish")]
    fn rejects_multiple_finishes() {
        assert_chat_interpreter_events(&[
            StreamPart::StepStart,
            StreamPart::finish(FinishReason::Stop, TokenUsage::simple(1, 1)),
            StreamPart::finish(FinishReason::Stop, TokenUsage::simple(1, 1)),
        ]);
    }
}
