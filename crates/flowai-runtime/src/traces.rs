use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use agent_fw_eval::TraceRecord;
use async_trait::async_trait;

#[derive(Debug, thiserror::Error)]
pub enum TraceSinkError {
    #[error("trace sink storage failed: {0}")]
    Storage(String),
}

#[async_trait]
pub trait TraceSink: Send + Sync {
    async fn record_trace(&self, trace: TraceRecord) -> Result<(), TraceSinkError>;

    async fn get_trace(&self, trace_id: &str) -> Result<Option<TraceRecord>, TraceSinkError>;

    async fn list_traces(
        &self,
        filter: TraceListFilter,
    ) -> Result<Vec<TraceRecord>, TraceSinkError>;
}

#[derive(Debug, Clone, Default)]
pub struct TraceListFilter {
    pub eval_run_id: Option<String>,
    pub test_case_id: Option<String>,
    pub thread_id: Option<String>,
}

#[derive(Debug, Default)]
pub struct NoopTraceSink;

#[async_trait]
impl TraceSink for NoopTraceSink {
    async fn record_trace(&self, _trace: TraceRecord) -> Result<(), TraceSinkError> {
        Ok(())
    }

    async fn get_trace(&self, _trace_id: &str) -> Result<Option<TraceRecord>, TraceSinkError> {
        Ok(None)
    }

    async fn list_traces(
        &self,
        _filter: TraceListFilter,
    ) -> Result<Vec<TraceRecord>, TraceSinkError> {
        Ok(vec![])
    }
}

#[derive(Debug, Default)]
pub struct RecordingTraceSink {
    traces: Mutex<BTreeMap<String, TraceRecord>>,
}

impl RecordingTraceSink {
    pub fn new() -> Self {
        Self::default()
    }

    fn matches_filter(trace: &TraceRecord, filter: &TraceListFilter) -> bool {
        if let Some(eval_run_id) = filter.eval_run_id.as_deref() {
            if trace.scope.eval_run_id.as_ref().map(|id| id.as_str()) != Some(eval_run_id) {
                return false;
            }
        }
        if let Some(test_case_id) = filter.test_case_id.as_deref() {
            if trace.scope.test_case_id.as_ref().map(|id| id.as_str()) != Some(test_case_id) {
                return false;
            }
        }
        if let Some(thread_id) = filter.thread_id.as_deref() {
            if trace.scope.thread_id.as_ref().map(|id| id.as_str()) != Some(thread_id) {
                return false;
            }
        }
        true
    }
}

#[async_trait]
impl TraceSink for RecordingTraceSink {
    async fn record_trace(&self, trace: TraceRecord) -> Result<(), TraceSinkError> {
        let mut guard = self
            .traces
            .lock()
            .map_err(|err| TraceSinkError::Storage(err.to_string()))?;
        guard.insert(trace.trace_id.clone(), trace);
        Ok(())
    }

    async fn get_trace(&self, trace_id: &str) -> Result<Option<TraceRecord>, TraceSinkError> {
        let guard = self
            .traces
            .lock()
            .map_err(|err| TraceSinkError::Storage(err.to_string()))?;
        Ok(guard.get(trace_id).cloned())
    }

    async fn list_traces(
        &self,
        filter: TraceListFilter,
    ) -> Result<Vec<TraceRecord>, TraceSinkError> {
        let guard = self
            .traces
            .lock()
            .map_err(|err| TraceSinkError::Storage(err.to_string()))?;
        Ok(guard
            .values()
            .filter(|trace| Self::matches_filter(trace, &filter))
            .cloned()
            .collect())
    }
}

pub type SharedTraceSink = Arc<dyn TraceSink>;
