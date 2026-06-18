import { describe, expect, test } from "bun:test";
import type { AppScope } from "../domain/scope";
import {
  type ChatRuntimeAdapter,
  type ConnectRuntimeAdapter,
  type EvalRuntimeAdapter,
  hasDescendingUpdatedAtOrder,
  hasStableThreadShape,
  makeAdapterLawFixture,
  type TestBuilderRuntimeAdapter,
  type TestRuntimeAdapter,
  type ThreadSummary,
} from "./adapters";

const scope: AppScope = { type: "workspace", workspaceId: "workspace-a" };

const orderedThreads: readonly ThreadSummary[] = [
  { id: "thread-2", title: "Second", updatedAt: "2026-04-13T12:00:00.000Z" },
  { id: "thread-1", title: "First", updatedAt: "2026-04-13T10:00:00.000Z" },
];

const chatAdapterFixture: ChatRuntimeAdapter = {
  async listThreads(inputScope) {
    expect(inputScope).toEqual(scope);
    return { _tag: "Ok", value: orderedThreads };
  },
  async getThread(_inputScope, threadId) {
    return {
      _tag: "Ok",
      value: {
        id: threadId,
        title: "Fixture",
        updatedAt: "2026-04-13T12:00:00.000Z",
      },
    };
  },
  async deleteThread() {
    return { _tag: "Ok", value: undefined };
  },
  async startChatStream() {
    return { _tag: "Ok", value: { abort: () => {} } };
  },
};

const testAdapterFixture: TestRuntimeAdapter<
  { id: string; updatedAt: string },
  { id: string; updatedAt: string; input: string }
> = {
  async listTests(inputScope) {
    expect(inputScope).toEqual(scope);
    return {
      _tag: "Ok",
      value: [
        { id: "test-2", updatedAt: "2026-04-13T12:00:00.000Z" },
        { id: "test-1", updatedAt: "2026-04-13T10:00:00.000Z" },
      ],
    };
  },
  async getTest(_inputScope, testId) {
    return {
      _tag: "Ok",
      value: {
        id: testId,
        updatedAt: "2026-04-13T12:00:00.000Z",
        input: "Fixture test case",
      },
    };
  },
};

const testBuilderAdapterFixture: TestBuilderRuntimeAdapter<
  { sessionId: string; updatedAt: string },
  { id: string; updatedAt: string },
  { request: { threadId: string } },
  { kind: string }
> = {
  async getSession(sessionId) {
    return {
      _tag: "Ok",
      value: {
        sessionId,
        updatedAt: "2026-04-13T12:00:00.000Z",
      },
    };
  },
  async clearSession() {
    return { _tag: "Ok", value: undefined };
  },
  async saveSession(input) {
    return {
      _tag: "Ok",
      value: {
        id: input.sessionId,
        updatedAt: "2026-04-13T12:00:00.000Z",
      },
    };
  },
  async startBuilderStream() {
    return { _tag: "Ok", value: { abort: () => {} } };
  },
};

const evalAdapterFixture: EvalRuntimeAdapter<
  { id: string; updatedAt: string },
  { id: string; updatedAt: string; status: string }
> = {
  async listRuns(inputScope) {
    expect(inputScope).toEqual(scope);
    return {
      _tag: "Ok",
      value: [
        { id: "eval-2", updatedAt: "2026-04-13T12:00:00.000Z" },
        { id: "eval-1", updatedAt: "2026-04-13T10:00:00.000Z" },
      ],
    };
  },
  async getRun(_inputScope, runId) {
    return {
      _tag: "Ok",
      value: {
        id: runId,
        updatedAt: "2026-04-13T12:00:00.000Z",
        status: "running",
      },
    };
  },
};

const connectAdapterFixture: ConnectRuntimeAdapter<
  { id: string; updatedAt: string; name: string },
  { id: string; updatedAt: string; name: string }
> = {
  async listSources(inputScope) {
    expect(inputScope).toEqual(scope);
    return {
      _tag: "Ok",
      value: [
        { id: "source-2", updatedAt: "2026-04-13T12:00:00.000Z", name: "Warehouse" },
        { id: "source-1", updatedAt: "2026-04-13T10:00:00.000Z", name: "Primary" },
      ],
    };
  },
  async getSource(_inputScope, sourceId) {
    return {
      _tag: "Ok",
      value: {
        id: sourceId,
        updatedAt: "2026-04-13T12:00:00.000Z",
        name: "Warehouse",
      },
    };
  },
  async deleteSource() {
    return { _tag: "Ok", value: undefined };
  },
  async listTables() {
    return { _tag: "Ok", value: [] };
  },
  async getTableDetail() {
    return { _tag: "Ok", value: { tableName: "orders" } };
  },
  async listDocuments() {
    return { _tag: "Ok", value: [] };
  },
  async browseKnowledge() {
    return { _tag: "Ok", value: [] };
  },
  async createExploreThread() {
    return { _tag: "Ok", value: { id: "thread-1" } };
  },
  async getAdminStatus() {
    return { _tag: "Ok", value: { databases: [] } };
  },
  async runMigrations() {
    return { _tag: "Ok", value: [] };
  },
  async purgeDatabase() {
    return { _tag: "Ok", value: { success: true } };
  },
};

describe("runtime adapter contracts", () => {
  test("validates descending updatedAt ordering", async () => {
    const result = await chatAdapterFixture.listThreads(scope);
    expect(result._tag).toBe("Ok");
    if (result._tag === "Ok") {
      expect(hasDescendingUpdatedAtOrder(result.value)).toBe(true);
    }
  });

  test("validates stable thread shape helper", () => {
    expect(hasStableThreadShape(orderedThreads, orderedThreads)).toBe(true);
    expect(
      hasStableThreadShape(orderedThreads, [
        { id: "thread-2", title: "Changed", updatedAt: "2026-04-13T12:00:00.000Z" },
        { id: "thread-1", title: "First", updatedAt: "2026-04-13T10:00:00.000Z" },
      ])
    ).toBe(false);
  });

  test("materializes adapter law fixtures as typed results", () => {
    const okLaw = makeAdapterLawFixture("listThreads sorted desc", true, {
      ids: orderedThreads.map((thread) => thread.id),
    });
    expect(okLaw._tag).toBe("Ok");

    const failingLaw = makeAdapterLawFixture("listThreads sorted desc", false, {
      ids: ["thread-1", "thread-2"],
    });
    expect(failingLaw._tag).toBe("Err");
    if (failingLaw._tag === "Err") {
      expect(failingLaw.error.code).toBe("UNSUPPORTED_SHAPE");
    }
  });

  test("keeps test runtime adapter contracts explicit", async () => {
    const result = await testAdapterFixture.listTests(scope);
    expect(result._tag).toBe("Ok");
    if (result._tag === "Ok") {
      const law = makeAdapterLawFixture(
        "listTests returns stable host-defined ordering",
        result.value[0]?.id === "test-2" && result.value[1]?.id === "test-1",
        { ids: result.value.map((testCase) => testCase.id) }
      );
      expect(law._tag).toBe("Ok");
    }
  });

  test("keeps eval runtime adapter contracts explicit", async () => {
    const result = await evalAdapterFixture.listRuns(scope);
    expect(result._tag).toBe("Ok");
    if (result._tag === "Ok") {
      const law = makeAdapterLawFixture(
        "listRuns returns stable host-defined ordering",
        result.value[0]?.id === "eval-2" && result.value[1]?.id === "eval-1",
        { ids: result.value.map((run) => run.id) }
      );
      expect(law._tag).toBe("Ok");
    }

    const detail = await evalAdapterFixture.getRun(scope, "eval-2");
    expect(detail._tag).toBe("Ok");
  });

  test("keeps connect runtime adapter contracts explicit", async () => {
    const result = await connectAdapterFixture.listSources(scope);
    expect(result._tag).toBe("Ok");
    if (result._tag === "Ok") {
      const law = makeAdapterLawFixture(
        "listSources returns stable host-defined ordering",
        result.value[0]?.id === "source-2" && result.value[1]?.id === "source-1",
        { ids: result.value.map((source) => source.id) }
      );
      expect(law._tag).toBe("Ok");
    }

    const created = await connectAdapterFixture.createExploreThread(scope, {
      title: "Data Exploration",
    });
    expect(created._tag).toBe("Ok");
  });

  test("keeps test builder runtime adapter contracts explicit", async () => {
    const session = await testBuilderAdapterFixture.getSession("builder-session-1");
    expect(session._tag).toBe("Ok");

    const saved = await testBuilderAdapterFixture.saveSession({
      sessionId: "builder-session-1",
      status: "draft",
      userPrompt: "Create a test case",
      structuredGroundTruth: { kind: "flat" },
    });
    expect(saved._tag).toBe("Ok");

    const stream = await testBuilderAdapterFixture.startBuilderStream(
      { request: { threadId: "builder-session-1" } },
      { signal: new AbortController().signal }
    );
    expect(stream._tag).toBe("Ok");
  });
});
