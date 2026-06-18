import { index, layout, type RouteConfig, route } from "@react-router/dev/routes";

export default [
  // Index route — redirect to playground
  index("routes/_index.tsx"),

  // Runtime/workspace overview
  route("workspace", "routes/workspace.tsx"),

  // Playground routes (chat interface)
  layout("routes/playground/_layout.tsx", [
    route("playground", "routes/playground/_index.tsx"),
    route("playground/:threadId", "routes/playground/$threadId.tsx"),
  ]),

  // /chat aliases — sidebar and cross-links use /chat/:threadId
  layout("routes/chat/_layout.tsx", [
    route("chat", "routes/chat/_index.tsx"),
    route("chat/:threadId", "routes/chat/$threadId.tsx"),
  ]),

  // Tests routes with layout
  layout("routes/tests/_layout.tsx", [
    route("tests", "routes/tests/_index.tsx"),
    route("tests/new", "routes/tests/new.tsx"),
    route("tests/:testCaseId", "routes/tests/$testCaseId.tsx"),
  ]),

  // Eval routes with layout
  layout("routes/evals/_layout.tsx", [
    route("evals", "routes/evals/_index.tsx"),
    route("evals/new", "routes/evals/new.tsx"),
    route("evals/:evalId", "routes/evals/$evalId.tsx"),
    route("evals/:evalId/cases/:testCaseId", "routes/evals/$evalId.cases.$testCaseId.tsx"),
    route(
      "evals/:evalId/cases/:testCaseId/traces/:traceId",
      "routes/evals/$evalId.cases.$testCaseId.traces.$traceId.tsx"
    ),
  ]),

  // Runs / traces / activity + approval inbox
  layout("routes/runs/_layout.tsx", [
    route("runs", "routes/runs/_index.tsx"),
    route("runs/approvals", "routes/runs/approvals.tsx"),
    route("runs/:runId", "routes/runs/$runId.tsx"),
  ]),

  // Connect routes (data exploration & knowledge management)
  layout("routes/connect/_layout.tsx", [
    route("connect", "routes/connect/_index.tsx"),
    route("connect/sources/new", "routes/connect/sources.new.tsx"),
    route("connect/sources/:sourceId", "routes/connect/sources.$sourceId.tsx"),
    route("connect/import", "routes/connect/import.tsx"),
    route("connect/discovery", "routes/connect/discovery.tsx"),
    route("connect/profiling", "routes/connect/profiling.tsx"),
    route("connect/knowledge", "routes/connect/knowledge.tsx"),
    route("connect/search", "routes/connect/search.tsx"),
    route("connect/tools", "routes/connect/tools.tsx"),
  ]),
] satisfies RouteConfig;
