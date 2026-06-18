//! KV key functions for workspace store entities.
//!
//! Each entity type has a key function (id → key) and a constant for its index.
//! Index keys store `EntityIndex` structs that list all entity IDs per tenant.
//!
//! # Key Schema
//!
//! | Entity      | Key Pattern                  | Index Key              |
//! |-------------|------------------------------|------------------------|
//! | Thread      | `thread:{id}`                | `threads:index`        |
//! | Messages    | `thread:{id}:messages`       | (nested under thread)  |
//! | ThreadFiles | `files:{thread_id}`          | (nested under thread)  |
//! | File        | `file:{id}`                  | (standalone entity)    |
//! | TestCase    | `test_case:{id}`             | `test_cases:index`     |
//! | EvalRun     | `eval_run:{id}`              | `eval_runs:index`      |
//! | EvalResults | `eval_run:{id}:results`      | (nested under run)     |
//! | EvalSet     | `eval:test-cases:{id}`       | `eval:test-cases:index`|
//! | EvalForks   | `eval:run:{id}:forks:{tc}`   | (nested under run)     |
//! | DataSource  | `data_source:{id}`           | `data_sources:index`   |
//! | Workspace   | `workspace:{id}`             | `workspaces:index`     |

// =============================================================================
// Index Constants
// =============================================================================

pub const THREADS_INDEX: &str = "threads:index";
pub const TEST_CASES_INDEX: &str = "test_cases:index";
pub const EVAL_RUNS_INDEX: &str = "eval_runs:index";
pub const EVAL_TEST_CASE_SETS_INDEX: &str = "eval:test-cases:index";
pub const DATA_SOURCES_INDEX: &str = "data_sources:index";
pub const WORKSPACES_INDEX: &str = "workspaces:index";

// =============================================================================
// Key Functions
// =============================================================================

pub fn thread(id: &str) -> String {
    format!("thread:{}", id)
}

pub fn thread_messages(id: &str) -> String {
    format!("thread:{}:messages", id)
}

pub fn thread_cost(id: &str) -> String {
    format!("thread:{}:cost", id)
}

pub fn thread_latency(id: &str) -> String {
    format!("thread:{}:latency", id)
}

pub fn thread_files(id: &str) -> String {
    format!("files:{}", id)
}

pub fn stored_file(id: &str) -> String {
    format!("file:{}", id)
}

pub fn test_case(id: &str) -> String {
    format!("test_case:{}", id)
}

pub fn eval_run(id: &str) -> String {
    format!("eval_run:{}", id)
}

pub fn eval_run_results(id: &str) -> String {
    format!("eval_run:{}:results", id)
}

pub fn eval_test_case_set(id: &str) -> String {
    format!("eval:test-cases:{}", id)
}

pub fn eval_forks(eval_run_id: &str, test_case_id: &str) -> String {
    format!("eval:run:{}:forks:{}", eval_run_id, test_case_id)
}

pub fn data_source(id: &str) -> String {
    format!("data_source:{}", id)
}

pub fn workspace(id: &str) -> String {
    format!("workspace:{}", id)
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thread_key_format() {
        assert_eq!(thread("abc123"), "thread:abc123");
    }

    #[test]
    fn messages_key_format() {
        assert_eq!(thread_messages("abc123"), "thread:abc123:messages");
    }

    #[test]
    fn thread_cost_key_format() {
        assert_eq!(thread_cost("abc123"), "thread:abc123:cost");
    }

    #[test]
    fn thread_latency_key_format() {
        assert_eq!(thread_latency("abc123"), "thread:abc123:latency");
    }

    #[test]
    fn thread_files_key_format() {
        assert_eq!(thread_files("abc123"), "files:abc123");
    }

    #[test]
    fn stored_file_key_format() {
        assert_eq!(stored_file("f-1"), "file:f-1");
    }

    #[test]
    fn test_case_key_format() {
        assert_eq!(test_case("tc-1"), "test_case:tc-1");
    }

    #[test]
    fn eval_run_key_format() {
        assert_eq!(eval_run("run-1"), "eval_run:run-1");
    }

    #[test]
    fn eval_results_key_format() {
        assert_eq!(eval_run_results("run-1"), "eval_run:run-1:results");
    }

    #[test]
    fn eval_test_case_set_key_format() {
        assert_eq!(eval_test_case_set("set-1"), "eval:test-cases:set-1");
    }

    #[test]
    fn eval_forks_key_format() {
        assert_eq!(eval_forks("run-1", "tc-1"), "eval:run:run-1:forks:tc-1");
    }

    #[test]
    fn data_source_key_format() {
        assert_eq!(data_source("ds-1"), "data_source:ds-1");
    }

    #[test]
    fn workspace_key_format() {
        assert_eq!(workspace("ws-1"), "workspace:ws-1");
    }
}
