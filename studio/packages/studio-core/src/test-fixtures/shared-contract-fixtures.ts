export const commandCardApprovalDslFixture = JSON.stringify({
  components: [
    {
      name: "CommandCard",
      props: {
        planId: "plan-123",
        title: "Scenario Plan: 1 Action, 42 Products",
        description: "Apply the requested price change.",
        attributes: [
          {
            label: "Products",
            value: "42",
            section: "metrics",
            cardStyle: "stat-card",
          },
          {
            label: "Warnings",
            value: "Review pricing thresholds",
            section: "context",
            collapsible: true,
            defaultExpanded: true,
          },
        ],
        actions: [
          {
            id: "proceed_plan",
            label: "Proceed",
            variant: "primary",
          },
          {
            id: "modify_plan",
            label: "Modify",
            variant: "secondary",
          },
          {
            id: "cancel_plan",
            label: "Cancel",
            variant: "danger",
          },
        ],
      },
    },
  ],
});

export const commandCardReadOnlyDslFixture = JSON.stringify({
  components: [
    {
      name: "CommandCard",
      props: {
        planId: "plan-456",
        title: "Execution Complete",
        description: "The approved plan has already been applied.",
        attributes: [
          {
            label: "Outcome",
            value: "Applied successfully",
            section: "context",
          },
        ],
        actions: [],
      },
    },
  ],
});

export const commandCardUnsupportedDslFixture = JSON.stringify({
  components: [
    {
      name: "SweepCard",
      props: {
        title: "Unsupported",
      },
    },
  ],
});

export const malformedCommandCardDslFixture = '{"components":[';

export const chatBackendAssistantMessageFixture = {
  id: "msg-1",
  role: "assistant",
  content: null,
  parts: [
    {
      type: "tool-invocation",
      toolInvocationId: "call-1",
      toolName: "buildPlan",
      state: "result",
      args: { ok: true },
      result: { planId: "plan-123" },
    },
    {
      type: "flow-ui",
      dsl: commandCardApprovalDslFixture,
    },
    {
      type: "tool-progress",
      toolName: "buildPlan",
      phaseIndex: 1,
      totalPhases: 3,
      label: "validated",
    },
    {
      type: "unknown-part",
      value: 1,
    },
  ],
} as const;

export const chatUiPersistedMessageFixture = {
  id: "ui-2",
  role: "assistant",
  parts: [
    { type: "text", text: "world" },
    { type: "flow-ui", dsl: commandCardReadOnlyDslFixture },
  ],
  metadata: { createdAt: "2026-04-13T12:00:00.000Z" },
} as const;

export const chatHistoryFixture = [
  { role: "system", content: "system" },
  { role: "assistant", content: "assistant" },
] as const;

export const connectDiscoveryTablesFixture = [
  {
    schemaName: "public",
    tableName: "orders",
    columnNames: ["id", "customer_id", "status"],
    totalColumnCount: 3,
  },
] as const;

export const connectDiscoveryApiResultFixture = {
  _tag: "Ok",
  value: connectDiscoveryTablesFixture,
} as const;

export const connectDiscoveryErrorResultFixture = {
  _tag: "Err",
  error: {
    message: "boom",
  },
} as const;

export const evalSidebarRunsFixture = [
  {
    id: "eval-running",
    createdAt: "2026-04-14T10:00:00.000Z",
    resultCount: 0,
    config: { mode: "sequential", model: "gpt-4o-mini", passThreshold: 0.7 },
    status: { status: "running" },
  },
  {
    id: "eval-pass",
    createdAt: "2026-04-14T10:00:00.000Z",
    resultCount: 3,
    config: { mode: "testCaseBuilder", model: "claude-opus-4-6", passThreshold: 0.7 },
    status: { status: "completed", summary: { aggregateScore: 0.9 } },
  },
  {
    id: "eval-fail",
    createdAt: "2026-04-14T10:00:00.000Z",
    resultCount: 3,
    config: { mode: "customMode", model: "glm-4.5", passThreshold: 0.7 },
    status: { status: "completed", summary: { aggregateScore: 0.4 } },
  },
] as const;

export const testCaseApiFixture = {
  name: "Pricing test",
  description: "Updates prices for matching entities",
  input: "Set price",
  status: "active",
  expectedTrajectory: ["buildPlan", "apply"],
  trajectoryMode: "subset",
  groundTruth: null,
  structuredGroundTruth: {
    kind: "flat",
    expectedActions: [{ actionType: "UPDATE", payload: {} }],
    expectedFilters: {
      matchedFilters: {},
      numericFilters: {},
      booleanFilters: {},
      measureFilters: {},
    },
    expectedScope: {},
  },
  tags: ["pricing", "retail"],
} as const;

export const builderSessionApiFixture = {
  sessionId: "session-1",
  userPrompt: "Increase price for matching entities",
  composedTrajectory: [
    {
      toolName: "buildPlan",
      source: { type: "manual", reason: null },
      position: 0,
    },
  ],
  trajectorySources: [],
  trajectoryMode: "inOrder",
  groundTruth: null,
  structuredGroundTruth: {
    kind: "flat",
    expectedActions: [{ actionType: "UPDATE", payload: {} }],
    expectedFilters: {
      matchedFilters: {},
      numericFilters: {},
      booleanFilters: {},
      measureFilters: {},
    },
    expectedScope: {},
  },
  tags: [],
  createdAt: "2026-04-14T00:00:00.000Z",
  updatedAt: "2026-04-14T00:00:00.000Z",
} as const;

export const sseReplayBlocksFixture = [
  'id: evt-1\ndata: {"type":"text","text":"hello"}',
  'id: evt-2\ndata: {"type":"resync","message":"Stream lagged, please reconnect"}',
] as const;

export const sseTerminalBlockFixture =
  'id: evt-terminal\ndata: {"type":"done","status":"completed"}';
