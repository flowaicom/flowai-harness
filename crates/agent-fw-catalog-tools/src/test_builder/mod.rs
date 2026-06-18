//! Canonical test-case-builder tools.
//!
//! The framework now exposes a single builder story:
//! - `TrajectoryAuthoringToolKit` for trajectory/thread authoring
//! - `TestBuilderToolKit` for the full builder shell, including tool catalog
//!   listing and structured ground-truth authoring
//!
//! # Design
//!
//! - **Session-based**: A builder session is persisted in KV store
//! - **Trajectory composition**: Compose trajectories from thread segments or manual steps
//! - **Pure + Interpreter**: Types are pure data, tools are the interpreter

mod shared;
mod toolkit;
mod trajectory_toolkit;

pub type SessionError = agent_fw_eval::TestCaseBuilderError;
pub use agent_fw_eval::TestCaseBuilderSession;
pub use toolkit::{
    GetGroundTruthHandler, ListEvalToolsHandler, SetStructuredGroundTruthHandler,
    TestBuilderToolKit,
};
pub use trajectory_toolkit::TrajectoryAuthoringToolKit;
pub use trajectory_toolkit::{
    AddTrajectoryStepHandler, ComposeTrajectoryHandler, GetComposedTrajectoryHandler,
    ListThreadsHandler, MergeTraceSegmentHandler, RemoveTrajectoryStepHandler,
    ReorderTrajectoryStepHandler, SetTrajectoryModeHandler,
};
