use serde::{Deserialize, Serialize};

pub const AGENT_RUNTIME_CONTRACT_SCHEMA_V1: &str = "agent-runtime-contract/v1";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentRuntimeContractV1 {
    pub schema_version: String,
    pub agent_id: String,
    pub execution_modes: Vec<ExecutionModeV1>,
    #[serde(default)]
    pub provider_requirements: Vec<ProviderRequirementV1>,
    #[serde(default)]
    pub env_requirements: Vec<EnvRequirementV1>,
    pub resource_requirements: ResourceRequirementsV1,
    pub sandbox_requirements: SandboxRequirementsV1,
}

impl AgentRuntimeContractV1 {
    #[must_use]
    pub fn execution_mode(&self, kind: ExecutionModeKindV1) -> Option<&ExecutionModeV1> {
        self.execution_modes.iter().find(|mode| mode.kind == kind)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExecutionModeV1 {
    pub kind: ExecutionModeKindV1,
    #[serde(default)]
    pub command: Vec<String>,
    #[serde(default)]
    pub adapter: Option<String>,
    #[serde(default)]
    pub http: Option<ExecutionHttpEndpointV1>,
    pub cwd: String,
    #[serde(default)]
    pub timeout_seconds: Option<u64>,
    pub output_mode: ExecutionOutputModeV1,
}

impl ExecutionModeV1 {
    #[must_use]
    pub fn is_usable(&self) -> bool {
        !self.command.is_empty() || self.adapter.is_some()
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExecutionHttpEndpointV1 {
    #[serde(default)]
    pub base_url: Option<String>,
    #[serde(default)]
    pub chat_path: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionModeKindV1 {
    Chat,
    Eval,
    ToolCall,
    Serve,
    BatchEval,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionOutputModeV1 {
    StdioJson,
    ArtifactJson,
    NativeStream,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProviderRequirementV1 {
    pub id: String,
    pub kind: ProviderKindV1,
    pub purpose: ProviderPurposeV1,
    #[serde(default)]
    pub required: bool,
    #[serde(default)]
    pub capabilities: Vec<ProviderCapabilityV1>,
    #[serde(default)]
    pub selection_policy: ProviderSelectionPolicyV1,
    #[serde(default)]
    pub model: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKindV1 {
    OpenaiCompatible,
    Anthropic,
    LocalHttp,
    Embedded,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProviderPurposeV1 {
    PrimaryChat,
    EvalJudge,
    ProposalGeneration,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum ProviderCapabilityV1 {
    ToolCalling,
    StructuredOutput,
    Vision,
    LongContext,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProviderSelectionPolicyV1 {
    Fixed,
    #[default]
    Configurable,
    PreferredDefault,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EnvRequirementV1 {
    pub name: String,
    #[serde(default)]
    pub required: bool,
    #[serde(default)]
    pub secret: bool,
    pub purpose: String,
    #[serde(default)]
    pub applies_to_modes: Vec<ExecutionModeKindV1>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ResourceRequirementsV1 {
    pub cpu: CpuRequirementV1,
    pub memory: MemoryRequirementV1,
    #[serde(default)]
    pub gpu: Option<GpuRequirementV1>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CpuRequirementV1 {
    #[serde(default)]
    pub min_cores: Option<u32>,
    #[serde(default)]
    pub preferred_cores: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MemoryRequirementV1 {
    #[serde(default)]
    pub min_mb: Option<u64>,
    #[serde(default)]
    pub preferred_mb: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GpuRequirementV1 {
    pub min_count: u32,
    #[serde(default)]
    pub preferred_count: Option<u32>,
    pub backend: GpuBackendV1,
    #[serde(default)]
    pub min_vram_gb: Option<u32>,
    #[serde(default)]
    pub multi_gpu_supported: bool,
    #[serde(default)]
    pub allowed_device_ids: Vec<u32>,
    #[serde(default)]
    pub preferred_device_ids: Vec<u32>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GpuBackendV1 {
    Cuda,
    Rocm,
    Metal,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SandboxRequirementsV1 {
    #[serde(default)]
    pub writable_paths: Vec<String>,
    #[serde(default)]
    pub readable_paths: Vec<String>,
    pub network: NetworkRequirementsV1,
    #[serde(default)]
    pub required_binaries: Vec<String>,
    #[serde(default)]
    pub mounts: Vec<MountRequirementV1>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NetworkRequirementsV1 {
    pub mode: NetworkModeV1,
    #[serde(default)]
    pub allowed_hosts: Vec<String>,
    #[serde(default)]
    pub allowed_schemes: Vec<String>,
    #[serde(default)]
    pub allowed_ports: Vec<u16>,
    #[serde(default)]
    pub required_in_modes: Vec<ExecutionModeKindV1>,
    #[serde(default)]
    pub offline_fallback_available: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum NetworkModeV1 {
    None,
    Restricted,
    Open,
}

impl Default for NetworkModeV1 {
    fn default() -> Self {
        Self::None
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MountRequirementV1 {
    pub source: String,
    pub target: String,
    #[serde(default)]
    pub read_only: bool,
}
