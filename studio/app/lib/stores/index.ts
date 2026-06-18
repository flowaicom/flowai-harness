/**
 * Store module for agent-fw studio.
 *
 * Conflicting selector names across stores are re-exported with prefixed names.
 * Import directly from the specific store module for the original names.
 *
 * Conflicts resolved:
 * - selectResourceId: chat-store vs thread-store → kept from chat-store
 * - selectSetResourceId: chat-store vs thread-store → kept from chat-store
 * - selectIsLoading: thread-store vs settings-store → kept from thread-store (as selectThreadsLoading)
 */

// Catalog search store — no conflicts with above stores
export * from "./catalog-search";
// Chat store — all exports (includes selectResourceId, selectSetResourceId)
export * from "./chat-store";
// Eval store — no conflicts with above stores
export * from "./eval-store";
// Import pipeline store — no conflicts with above stores
export * from "./import-pipeline";
// Knowledge base store — no conflicts with above stores
export * from "./knowledge-base";
// Lifecycle bus — no conflicts
export * from "./lifecycle-bus";
// Profiling pipeline store — no conflicts with above stores
export * from "./profiling-pipeline";
// Schema explorer store — no conflicts with above stores
export * from "./schema-explorer";
// Session registry store — cross-cutting session tracker for Activity Center
export * from "./session-registry";
// Settings store — exclude selectIsLoading (conflicts with thread-store alias)
export {
  type AgentCustomEndpoint,
  type AgentCustomEndpoints,
  type AgentInfo,
  type AgentModelSelection,
  type AgentRole,
  type AgentSelectedModels,
  type FeatureFlags,
  type ModelConfig,
  type ModelConfigResponse,
  // Types
  type ModelKey,
  type ModelPricing,
  type ModelSettings,
  type ProviderKey,
  type ProviderModel,
  type ProviderSetting,
  type ProviderSettingKind,
  type ProviderSettingOption,
  type ProviderSettings,
  type SettingsStore,
  selectActiveModelConfigWorkspaceId,
  selectAgentCustomEndpoints,
  selectAgentModelForRole,
  // State selectors
  selectAgentModels,
  selectAgentSelectedModels,
  selectAgents,
  selectAvailableModels,
  selectCloseSettingsDialog,
  selectFeatureFlags,
  selectFullCsvExport,
  selectGetAgentModelConfig,
  selectGetModelsForProvider,
  selectGetProviderSettings,
  selectIsHydrated,
  selectIsLoaded,
  selectIsLoadingProviderModels,
  selectLoadModelConfig,
  selectLoadProviderModels,
  selectMaxTokens,
  selectModelSettings,
  selectNeondbApiKey,
  selectNeondbProjectId,
  selectOpenSettingsDialog,
  selectPiiScramble,
  selectProviderModels,
  selectProviderModelsError,
  selectProviderSettings,
  selectReasoningEffort,
  selectSetAgentCustomEndpoint,
  // Action selectors
  selectSetAgentModel,
  selectSetAgentSelectedModel,
  selectSetAllAgentModels,
  selectSetFeatureFlag,
  selectSetMaxTokens,
  selectSetNeondbApiKey,
  selectSetNeondbProjectId,
  selectSetProviderSetting,
  selectSetReasoningEffort,
  selectSetTheme,
  selectSetThinkingBudgetTokens,
  selectSettingsDialogOpen,
  selectSettingsError,
  selectShowLatencyPanel,
  selectTheme,
  selectThinkingBudgetTokens,
  selectToggleSettingsDialog,
  // Stores
  useAgentConfig,
  useAgentModel,
  useFeatureFlag,
  useSettings,
  useSettingsStore,
} from "./settings-store";
// Source catalog store — no conflicts with above stores
export * from "./source-catalog";
// Test suite store — no conflicts with above stores
export * from "./test-store";
// Thread store — exclude selectResourceId/selectSetResourceId/selectIsLoading (conflict with chat/settings)
// Use selectThreadResourceId, selectThreadsLoading directly instead.
export {
  selectActiveThread,
  selectActiveThreadId,
  selectAddThread,
  selectError,
  selectGetThread,
  selectHasThreads,
  selectIsReady,
  selectListState,
  selectRecentThreads,
  selectRemoveThread,
  selectResetThreads,
  selectSelectedThread,
  selectSelectedThreadId,
  selectSetActiveThread,
  selectSetListState,
  // Action selectors
  selectSetThreads,
  selectThreadCount,
  selectThreadLoadError,
  // Compatibility aliases (that don't conflict)
  selectThreadLoadPhase,
  selectThreadResourceId,
  // State selectors
  selectThreads,
  selectThreadsLoading,
  selectThreadsLoadPhase,
  selectThreadsReady,
  selectTouchThread,
  selectUpdateThread,
  type ThreadActions,
  type ThreadCatalog,
  type ThreadCatalogActions,
  type ThreadCatalogState,
  type ThreadListState,
  type ThreadState,
  type ThreadStore,
  type ThreadsActions,
  // Types
  type ThreadsState,
  type ThreadsStore,
  useThreadActions,
  useThreadCatalog,
  useThreadCatalogActions,
  useThreadStore,
  // Stores
  useThreads,
} from "./thread-store";
// Workspace store — no conflicts
export * from "./workspace-store";
