import type { CommandCardActionHandler } from "../domain/command-card";
import type { ApiError, ApiResult, ParseError, StreamError } from "../domain/errors";
import type { Result } from "../domain/result";
import type { AppScope } from "../domain/scope";
import type { StudioStorageAdapters } from "../storage/namespaced-storage";

export interface TimestampedEntity {
  readonly updatedAt: string;
}

export interface ThreadSummary extends TimestampedEntity {
  readonly id: string;
  readonly title: string | null;
}

export interface ThreadDetail extends ThreadSummary {
  readonly messageCount?: number;
}

export interface ChatStreamHandle {
  readonly abort: () => void;
}

export type WorkspaceKey = string;

export interface StudioStatus {
  readonly studioApiVersion: string;
  readonly supportedVersions: readonly string[];
  readonly status: string;
  readonly implementation?: Record<string, unknown>;
}

export interface WorkspaceSummary {
  readonly workspaceKey: WorkspaceKey;
  readonly displayName?: string;
  readonly default?: boolean;
  readonly metadata?: Record<string, unknown>;
}

export interface WorkspacesResponse {
  readonly defaultWorkspaceKey: WorkspaceKey;
  readonly workspaces: readonly WorkspaceSummary[];
}

export interface RuntimeSummary {
  readonly workspaceKey: WorkspaceKey;
  readonly runtime?: Record<string, unknown>;
  readonly tenant?: Record<string, unknown>;
  readonly providers?: readonly unknown[];
  readonly agents?: readonly AgentSummary[];
  readonly references?: readonly unknown[];
}

export interface CapabilitySummary {
  readonly id: string;
  readonly enabled: boolean;
  readonly scope: "local" | "enterprise" | string;
  readonly reason?: string;
  readonly requirements?: readonly string[];
}

export interface CapabilitiesResponse {
  readonly workspaceKey: WorkspaceKey;
  readonly capabilities: readonly CapabilitySummary[];
}

export interface AgentSummary {
  readonly agentId: string;
  readonly name: string;
  readonly role: string;
  readonly model: string;
  readonly stateful: boolean;
  readonly entrypoint: boolean;
  readonly tools?: readonly string[];
  readonly toolkits?: readonly string[];
  readonly routes?: readonly string[];
}

export interface AgentsResponse {
  readonly workspaceKey: WorkspaceKey;
  readonly agents: readonly AgentSummary[];
}

export interface ThreadMessage {
  readonly messageId: string;
  readonly threadId: string;
  readonly role: "user" | "assistant" | "system" | "tool" | string;
  readonly content: string;
  readonly metadata: Record<string, unknown>;
  readonly createdAt: string;
}

export interface StudioEvent<TPayload = unknown> {
  readonly schemaVersion: string;
  readonly workspaceKey: WorkspaceKey;
  readonly runId: string;
  readonly threadId: string;
  readonly agentId: string;
  readonly seq: number;
  readonly kind: string;
  readonly payload: TPayload;
}

export interface HarnessChatStreamInput {
  readonly agentId: string;
  readonly prompt: string;
  readonly threadId: string;
  readonly runId?: string;
  readonly metadata?: Record<string, unknown>;
  readonly handlers?: HarnessChatStreamHandlers;
}

export interface HarnessChatStreamHandlers<TEvent extends StudioEvent = StudioEvent> {
  readonly onEvent: (event: TEvent) => void;
  readonly onComplete: () => void;
  readonly onError: (error: ApiError | StreamError) => void;
  readonly onEventId?: (eventId: string) => void;
}

export interface ApprovalResponseInput {
  readonly outcome: "approve" | "reject" | "revise" | "approved" | "rejected";
  readonly feedback?: string;
  readonly reason?: string;
  readonly partial?: Record<string, unknown>;
}

export interface ApprovalSummary {
  readonly approvalId: string;
  readonly status: string;
  readonly kind?: string;
  readonly agentId?: string;
  readonly runId?: string;
  readonly title?: string;
  readonly createdAt?: string;
  readonly payload?: Record<string, unknown>;
}

export interface ToolkitSummary {
  readonly toolkitId: string;
  readonly name: string;
  readonly enabled: boolean;
  readonly tools?: readonly string[];
  readonly metadata?: Record<string, unknown>;
}

export interface ToolSummary {
  readonly toolId: string;
  readonly name: string;
  readonly origin: "runtime" | "toolkit" | "host" | string;
  readonly approval?: "never" | "always" | "dynamic" | string;
  readonly description?: string;
  readonly toolkit?: string;
  readonly inputSchema?: Record<string, unknown>;
  readonly outputSchema?: Record<string, unknown>;
}

export interface ToolExecutionInput {
  readonly toolId: string;
  readonly input?: Record<string, unknown>;
  readonly agentId?: string;
}

export interface ToolExecutionResult {
  readonly toolId: string;
  readonly status: string;
  readonly output?: unknown;
}

export interface DataSourceSummary {
  readonly sourceId: string;
  readonly name: string;
  readonly kind?: string;
  readonly status?: string;
  readonly metadata?: Record<string, unknown>;
}

export interface ListTablesInput {
  readonly sourceId?: string;
  readonly schema?: string;
  readonly signal?: AbortSignal;
}

export interface TableSummary {
  readonly tableName: string;
  readonly schemaName?: string;
  readonly columnCount?: number;
  readonly metadata?: Record<string, unknown>;
}

export interface TableDetailInput {
  readonly tableName: string;
  readonly schema?: string;
  readonly sourceId?: string;
}

export interface TableDetail extends TableSummary {
  readonly columns?: readonly unknown[];
  readonly sampleRows?: readonly Record<string, unknown>[];
}

export interface ListSchemasInput {
  readonly sourceId?: string;
  readonly signal?: AbortSignal;
}

export interface TableColumnsInput extends TableDetailInput {
  readonly signal?: AbortSignal;
}

export interface SampleRowsInput extends TableDetailInput {
  readonly limit?: number;
  readonly signal?: AbortSignal;
}

export interface SampleRowsResult {
  readonly rows: readonly Record<string, unknown>[];
  readonly metadata?: Record<string, unknown>;
}

export interface DocumentSummary {
  readonly documentId: string;
  readonly title: string;
  readonly sourceId?: string;
  readonly metadata?: Record<string, unknown>;
}

export interface BrowseKnowledgeInput {
  readonly sourceId?: string;
  readonly query?: string;
  readonly limit?: number;
}

export interface KnowledgeItem {
  readonly itemId: string;
  readonly title?: string;
  readonly content?: string;
  readonly metadata?: Record<string, unknown>;
}

export interface MetricSummary {
  readonly metricId: string;
  readonly name: string;
  readonly description?: string;
  readonly metricType?: string;
  readonly tags?: readonly string[];
  readonly metadata?: Record<string, unknown>;
}

export interface CatalogSearchInput {
  readonly query: string;
  readonly limit?: number;
  readonly mode?: string;
  readonly sourceId?: string;
  readonly filters?: Record<string, unknown>;
}

export interface CatalogSearchResult {
  readonly items: readonly unknown[];
  readonly metadata?: Record<string, unknown>;
}

export interface SearchToolInput {
  readonly mode: string;
  readonly query: string;
  readonly limit?: number;
  readonly filters?: Record<string, unknown>;
}

export interface SearchToolResult {
  readonly rows: readonly unknown[];
  readonly metadata?: Record<string, unknown>;
}

export interface ProfileEstimateInput {
  readonly sourceId?: string;
  readonly databaseId?: string;
  readonly schemaName?: string;
  readonly tables?: readonly string[];
  readonly modelId?: string;
  readonly sampleSize?: number;
}

export interface ProfileEstimateResult {
  readonly estimate: unknown;
  readonly metadata?: Record<string, unknown>;
}

export interface DataEventStreamHandlers<TEvent = unknown> {
  readonly onEvent: (event: TEvent) => void;
  readonly onComplete: () => void;
  readonly onError: (error: ApiError | StreamError) => void;
}

export interface DataEventStreamHandle {
  readonly abort: () => void;
}

export interface ProfileTableInput extends ProfileEstimateInput {
  readonly tableName: string;
  readonly handlers?: DataEventStreamHandlers;
}

export interface ProfileDatabaseInput extends ProfileEstimateInput {
  readonly handlers?: DataEventStreamHandlers;
}

export interface KnowledgeIngestInput {
  readonly source: Record<string, unknown>;
  readonly extractKnowledge?: boolean;
  readonly handlers?: DataEventStreamHandlers;
}

export interface TestSummary extends TimestampedEntity {
  readonly testId: string;
  readonly name: string;
  readonly status?: string;
}

export interface TestDetail extends TestSummary {
  readonly input?: string;
  readonly groundTruth?: unknown;
  readonly metadata?: Record<string, unknown>;
}

export interface CreateTestInput {
  readonly name: string;
  readonly input: string;
  readonly groundTruth?: unknown;
  readonly metadata?: Record<string, unknown>;
}

export interface EvalRunSummary extends TimestampedEntity {
  readonly runId: string;
  readonly status: string;
  readonly score?: number;
}

export interface EvalRunDetail extends EvalRunSummary {
  readonly cases?: readonly unknown[];
  readonly metadata?: Record<string, unknown>;
}

export interface StartEvalRunInput {
  readonly evalId?: string;
  readonly testIds?: readonly string[];
  readonly mode?: string;
  readonly metadata?: Record<string, unknown>;
}

export interface RunSummary extends TimestampedEntity {
  readonly runId: string;
  readonly status: string;
  readonly kind?: string;
}

export interface RunDetail extends RunSummary {
  readonly events?: readonly StudioEvent[];
  readonly metadata?: Record<string, unknown>;
}

export interface HarnessDiscoveryRuntimeAdapter {
  getStatus(): Promise<ApiResult<StudioStatus>>;
  listWorkspaces(): Promise<ApiResult<WorkspacesResponse>>;
  getWorkspace(scope: AppScope): Promise<ApiResult<WorkspaceSummary>>;
  getRuntime(scope: AppScope): Promise<ApiResult<RuntimeSummary>>;
  getCapabilities(scope: AppScope): Promise<ApiResult<CapabilitiesResponse>>;
  listAgents(scope: AppScope): Promise<ApiResult<AgentsResponse>>;
}

export interface HarnessChatRuntimeAdapter<
  TThreadSummary extends ThreadSummary = ThreadSummary,
  TThreadDetail extends ThreadDetail = ThreadDetail,
  TMessage extends ThreadMessage = ThreadMessage,
  TStreamInput extends HarnessChatStreamInput = HarnessChatStreamInput,
  TStreamHandle extends ChatStreamHandle = ChatStreamHandle,
> extends ChatRuntimeAdapter<TThreadSummary, TThreadDetail, TStreamInput, TStreamHandle> {
  listThreadMessages(scope: AppScope, threadId: string): Promise<ApiResult<readonly TMessage[]>>;
}

export interface HarnessApprovalRuntimeAdapter<TApprovalSummary = ApprovalSummary> {
  respondToApproval(
    scope: AppScope,
    approvalId: string,
    input: ApprovalResponseInput
  ): Promise<ApiResult<TApprovalSummary>>;
}

export interface HarnessToolsRuntimeAdapter {
  listToolkits(scope: AppScope): Promise<ApiResult<readonly ToolkitSummary[]>>;
  listTools(scope: AppScope): Promise<ApiResult<readonly ToolSummary[]>>;
  listAgentTools(scope: AppScope, agentId: string): Promise<ApiResult<readonly ToolSummary[]>>;
  executeTool(scope: AppScope, input: ToolExecutionInput): Promise<ApiResult<ToolExecutionResult>>;
}

export interface HarnessConnectRuntimeAdapter {
  listDataSources(scope: AppScope): Promise<ApiResult<readonly DataSourceSummary[]>>;
  createDataSource?(scope: AppScope, input: unknown): Promise<ApiResult<never>>;
  updateDataSource?(scope: AppScope, sourceId: string, input: unknown): Promise<ApiResult<never>>;
  deleteDataSource?(scope: AppScope, sourceId: string): Promise<ApiResult<never>>;
  testDataSource?(scope: AppScope, input: unknown): Promise<ApiResult<never>>;
  listSchemas(scope: AppScope, input?: ListSchemasInput): Promise<ApiResult<readonly string[]>>;
  listTables(scope: AppScope, input?: ListTablesInput): Promise<ApiResult<readonly TableSummary[]>>;
  getTableDetail(scope: AppScope, input: TableDetailInput): Promise<ApiResult<TableDetail>>;
  getTableColumns(
    scope: AppScope,
    input: TableColumnsInput
  ): Promise<ApiResult<readonly unknown[]>>;
  sampleRows(scope: AppScope, input: SampleRowsInput): Promise<ApiResult<SampleRowsResult>>;
  estimateProfile(
    scope: AppScope,
    input: ProfileEstimateInput
  ): Promise<ApiResult<ProfileEstimateResult>>;
  startProfileTable(
    scope: AppScope,
    input: ProfileTableInput
  ): Promise<Result<DataEventStreamHandle, StreamError | ApiError>>;
  startProfileDatabase(
    scope: AppScope,
    input: ProfileDatabaseInput
  ): Promise<Result<DataEventStreamHandle, StreamError | ApiError>>;
  listDocuments(scope: AppScope): Promise<ApiResult<readonly DocumentSummary[]>>;
  browseKnowledge(
    scope: AppScope,
    input?: BrowseKnowledgeInput
  ): Promise<ApiResult<readonly KnowledgeItem[]>>;
  ingestKnowledge(
    scope: AppScope,
    input: KnowledgeIngestInput
  ): Promise<Result<DataEventStreamHandle, StreamError | ApiError>>;
  listMetrics(scope: AppScope): Promise<ApiResult<readonly MetricSummary[]>>;
  searchCatalog(
    scope: AppScope,
    input: CatalogSearchInput
  ): Promise<ApiResult<CatalogSearchResult>>;
  runSearchTool(scope: AppScope, input: SearchToolInput): Promise<ApiResult<SearchToolResult>>;
  startImport?(scope: AppScope, input: unknown): Promise<ApiResult<never>>;
}

export interface HarnessTestsRuntimeAdapter {
  listTests(scope: AppScope): Promise<ApiResult<readonly TestSummary[]>>;
  getTest(scope: AppScope, testId: string): Promise<ApiResult<TestDetail>>;
  createTest(scope: AppScope, input: CreateTestInput): Promise<ApiResult<TestDetail>>;
}

export interface HarnessEvalsRuntimeAdapter {
  listEvalRuns(scope: AppScope): Promise<ApiResult<readonly EvalRunSummary[]>>;
  getEvalRun(scope: AppScope, runId: string): Promise<ApiResult<EvalRunDetail>>;
  startEvalRun(scope: AppScope, input: StartEvalRunInput): Promise<ApiResult<EvalRunDetail>>;
}

export interface HarnessRunsRuntimeAdapter {
  listRuns(scope: AppScope): Promise<ApiResult<readonly RunSummary[]>>;
  getRun(scope: AppScope, runId: string): Promise<ApiResult<RunDetail>>;
  listRunEvents(scope: AppScope, runId: string): Promise<ApiResult<readonly StudioEvent[]>>;
}

export type FlowAIHarnessRuntimeAdapter = HarnessDiscoveryRuntimeAdapter &
  HarnessChatRuntimeAdapter &
  HarnessApprovalRuntimeAdapter &
  HarnessToolsRuntimeAdapter &
  HarnessConnectRuntimeAdapter &
  HarnessTestsRuntimeAdapter &
  HarnessEvalsRuntimeAdapter &
  HarnessRunsRuntimeAdapter;

export interface ChatRuntimeAdapter<
  TThreadSummary extends ThreadSummary = ThreadSummary,
  TThreadDetail extends ThreadDetail = ThreadDetail,
  TStreamInput = unknown,
  TStreamHandle extends ChatStreamHandle = ChatStreamHandle,
> {
  /**
   * Law: results are ordered by updatedAt desc.
   * Law: consecutive calls without mutation return structurally equivalent lists.
   * Law: expected failures resolve to ApiError, not raw thrown transport exceptions.
   */
  listThreads(scope: AppScope): Promise<ApiResult<readonly TThreadSummary[]>>;
  getThread(scope: AppScope, threadId: string): Promise<ApiResult<TThreadDetail>>;
  deleteThread(scope: AppScope, threadId: string): Promise<ApiResult<void>>;
  /**
   * Law: startup failures resolve to typed stream errors.
   * Law: event order is preserved.
   * Law: no events are emitted after abort.
   */
  startChatStream(
    input: TStreamInput,
    options: { readonly signal: AbortSignal; readonly scope: AppScope }
  ): Promise<Result<TStreamHandle, StreamError | ApiError>>;
  readonly onCommandCardAction?: CommandCardActionHandler;
}

export interface EvalRuntimeAdapter<TRunSummary = unknown, TRunDetail = unknown> {
  /**
   * Law: list order is host-defined but stable across consecutive calls without mutation.
   * Law: expected failures resolve to ApiError, not raw thrown transport exceptions.
   */
  listRuns(
    scope: AppScope,
    params?: { readonly testCaseId?: string }
  ): Promise<ApiResult<readonly TRunSummary[]>>;
  /**
   * Law: expected failures resolve to ApiError, not raw thrown transport exceptions.
   */
  getRun(scope: AppScope, runId: string): Promise<ApiResult<TRunDetail>>;
}

export interface TestRuntimeAdapter<TTestSummary = unknown, TTestDetail = unknown> {
  /**
   * Law: list order is host-defined but stable across consecutive calls without mutation.
   * Law: expected failures resolve to ApiError, not raw thrown transport exceptions.
   */
  listTests(scope: AppScope): Promise<ApiResult<readonly TTestSummary[]>>;
  /**
   * Law: expected failures resolve to ApiError, not raw thrown transport exceptions.
   */
  getTest(scope: AppScope, testId: string): Promise<ApiResult<TTestDetail>>;
}

export interface TestBuilderSaveInput<TGroundTruth = unknown> {
  readonly sessionId: string;
  readonly status?: string;
  readonly userPrompt?: string;
  readonly structuredGroundTruth?: TGroundTruth;
  readonly groundTruth?: string;
}

export interface TestBuilderRuntimeAdapter<
  TSession = unknown,
  TSavedTest = unknown,
  TStreamInput = unknown,
  TGroundTruth = unknown,
  TStreamHandle extends ChatStreamHandle = ChatStreamHandle,
> {
  /**
   * Law: expected failures resolve to ApiError, not raw thrown transport exceptions.
   */
  getSession(sessionId: string): Promise<ApiResult<TSession>>;
  /**
   * Law: expected failures resolve to ApiError, not raw thrown transport exceptions.
   */
  clearSession(sessionId: string): Promise<ApiResult<void>>;
  /**
   * Law: expected failures resolve to ApiError, not raw thrown transport exceptions.
   */
  saveSession(input: TestBuilderSaveInput<TGroundTruth>): Promise<ApiResult<TSavedTest>>;
  /**
   * Law: startup failures resolve to typed stream errors.
   * Law: event order is preserved.
   * Law: no events are emitted after abort.
   */
  startBuilderStream(
    input: TStreamInput,
    options: { readonly signal: AbortSignal }
  ): Promise<Result<TStreamHandle, StreamError | ApiError>>;
}

export interface ConnectRuntimeAdapter<TSourceSummary = unknown, TSourceDetail = unknown> {
  /**
   * Law: list order is host-defined but stable across consecutive calls without mutation.
   * Law: expected failures resolve to ApiError, not raw thrown transport exceptions.
   */
  listSources(scope: AppScope): Promise<ApiResult<readonly TSourceSummary[]>>;
  /**
   * Law: expected failures resolve to ApiError, not raw thrown transport exceptions.
   */
  getSource(scope: AppScope, sourceId: string): Promise<ApiResult<TSourceDetail>>;
  /**
   * Law: expected failures resolve to ApiError, not raw thrown transport exceptions.
   */
  deleteSource(scope: AppScope, sourceId: string): Promise<ApiResult<void>>;
  listTables(
    scope: AppScope,
    params?: { readonly schema?: string; readonly signal?: AbortSignal }
  ): Promise<ApiResult<readonly unknown[]>>;
  getTableDetail(
    scope: AppScope,
    tableName: string,
    params?: { readonly schema?: string }
  ): Promise<ApiResult<unknown>>;
  listDocuments(scope: AppScope): Promise<ApiResult<readonly unknown[]>>;
  browseKnowledge(scope: AppScope): Promise<ApiResult<readonly unknown[]>>;
  createExploreThread(
    scope: AppScope,
    input: { readonly title: string }
  ): Promise<ApiResult<{ readonly id: string }>>;
  getAdminStatus(scope: AppScope): Promise<ApiResult<unknown>>;
  runMigrations(scope: AppScope, roles: readonly string[]): Promise<ApiResult<readonly unknown[]>>;
  purgeDatabase(scope: AppScope, role: string): Promise<ApiResult<unknown>>;
}

export interface FeatureStoreFactoryDeps<TRuntime> {
  readonly runtime: TRuntime;
  readonly getScope: () => AppScope | null;
}

export interface ChatStoreFactoryDeps<TRuntime extends ChatRuntimeAdapter = ChatRuntimeAdapter>
  extends FeatureStoreFactoryDeps<TRuntime> {
  readonly storage: StudioStorageAdapters;
}

export type ChatStoreFactory<TStore, TRuntime extends ChatRuntimeAdapter = ChatRuntimeAdapter> = (
  deps: ChatStoreFactoryDeps<TRuntime>
) => TStore;

export function hasDescendingUpdatedAtOrder<T extends TimestampedEntity>(
  items: readonly T[]
): boolean {
  for (let index = 1; index < items.length; index++) {
    const previous = Date.parse(items[index - 1].updatedAt);
    const current = Date.parse(items[index].updatedAt);
    if (Number.isNaN(previous) || Number.isNaN(current) || previous < current) {
      return false;
    }
  }
  return true;
}

export function hasStableThreadShape<T extends ThreadSummary>(
  previous: readonly T[],
  next: readonly T[]
): boolean {
  if (previous.length !== next.length) return false;
  return previous.every((item, index) => {
    const candidate = next[index];
    return (
      item.id === candidate.id &&
      item.title === candidate.title &&
      item.updatedAt === candidate.updatedAt
    );
  });
}

export function makeAdapterLawFixture(
  law: string,
  passed: boolean,
  details?: unknown
): Result<{ readonly law: string; readonly details?: unknown }, ParseError> {
  return passed
    ? {
        _tag: "Ok",
        value: { law, details },
      }
    : {
        _tag: "Err",
        error: {
          kind: "parse-error",
          code: "UNSUPPORTED_SHAPE",
          message: `Adapter law failed: ${law}`,
          issues: [law],
          details,
        },
      };
}
