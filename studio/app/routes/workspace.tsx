import type { AgentSummary, RuntimeSummary } from "@studio/core/runtime";
import { Badge, JsonPane, PagePanel, PageShell, StatCard, StudioHeader } from "@studio/ui";
import { BotIcon, DatabaseIcon, KeyRoundIcon, RouteIcon } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import { useHarnessRuntime } from "~/lib/runtime";
import { getFlowAIStudioConfig } from "~/lib/studio-config/flowai-config";

interface OverviewData {
  readonly runtime: RuntimeSummary | null;
  readonly agents: readonly AgentSummary[];
  readonly errors: readonly string[];
}

type OverviewState =
  | { readonly kind: "loading" }
  | { readonly kind: "ready"; readonly data: OverviewData };

export default function WorkspaceOverviewRoute() {
  const { adapter, scope, workspaceKey } = useHarnessRuntime();
  const config = useMemo(() => getFlowAIStudioConfig(), []);
  const [state, setState] = useState<OverviewState>({ kind: "loading" });

  useEffect(() => {
    let cancelled = false;

    async function load() {
      setState({ kind: "loading" });
      const [runtimeResult, agentsResult] = await Promise.all([
        adapter.getRuntime(scope),
        adapter.listAgents(scope),
      ]);

      if (cancelled) return;

      const errors: string[] = [];
      const runtime = runtimeResult._tag === "Ok" ? runtimeResult.value : null;
      if (runtimeResult._tag === "Err") errors.push(`Runtime: ${runtimeResult.error.message}`);

      const agents =
        agentsResult._tag === "Ok" ? agentsResult.value.agents : (runtime?.agents ?? []);
      if (agentsResult._tag === "Err") errors.push(`Agents: ${agentsResult.error.message}`);

      setState({
        kind: "ready",
        data: {
          runtime,
          agents,
          errors,
        },
      });
    }

    void load();

    return () => {
      cancelled = true;
    };
  }, [adapter, scope]);

  const data: OverviewData =
    state.kind === "ready"
      ? state.data
      : {
          runtime: null,
          agents: [],
          errors: [],
        };
  const entrypoint = data.agents.find((agent) => agent.entrypoint) ?? data.agents[0] ?? null;

  return (
    <PageShell>
      <StudioHeader
        eyebrow="Harness runtime"
        title={config.appName}
        description={`Workspace ${workspaceKey} exposes the runtime surfaces Studio can inspect, chat with, and extend through the harness Studio API.`}
        actions={
          state.kind === "loading" ? (
            <Badge tone="blue">Loading</Badge>
          ) : data.errors.length > 0 ? (
            <Badge tone="amber">Partial</Badge>
          ) : (
            <Badge tone="green">Ready</Badge>
          )
        }
      />

      <div className="grid gap-4 sm:grid-cols-3">
        <StatCard
          label="Workspace"
          value={workspaceKey}
          hint="Active resource scope"
          tone={data.errors.length > 0 ? "amber" : "green"}
        />
        <StatCard
          label="Agents"
          value={data.agents.length}
          meta={entrypoint ? `Entrypoint: ${entrypoint.name}` : "No entrypoint reported"}
          tone={entrypoint ? "blue" : "amber"}
        />
        <StatCard
          label="Providers"
          value={data.runtime?.providers?.length ?? 0}
          meta={config.studioApiVersion}
          tone="neutral"
        />
      </div>

      <div className="mt-6 grid gap-6 xl:grid-cols-2">
        <PagePanel className="p-5">
          <div className="mb-4 flex items-center gap-2">
            <BotIcon className="size-4 text-[var(--fg-4)]" />
            <h2 className="text-sm font-semibold text-[var(--fg-1)]">Agents</h2>
          </div>
          {data.agents.length > 0 ? (
            <div className="space-y-2">
              {data.agents.map((agent) => (
                <div
                  key={agent.agentId}
                  className="rounded-lg border border-[var(--layer-08)] bg-[var(--layer-03)] p-3"
                >
                  <div className="flex items-start justify-between gap-3">
                    <div className="min-w-0">
                      <div className="flex items-center gap-2">
                        <span className="truncate text-sm font-medium text-[var(--fg-1)]">
                          {agent.name}
                        </span>
                        {agent.entrypoint ? <Badge tone="blue">Entrypoint</Badge> : null}
                      </div>
                      <p className="mt-1 text-xs text-[var(--fg-5)]">
                        {agent.role} · {agent.model}
                      </p>
                    </div>
                    <Badge tone={agent.stateful ? "green" : "neutral"}>
                      {agent.stateful ? "Stateful" : "Stateless"}
                    </Badge>
                  </div>
                  {(agent.routes?.length ?? 0) > 0 || (agent.tools?.length ?? 0) > 0 ? (
                    <div className="mt-3 flex flex-wrap gap-1.5">
                      {agent.routes?.map((route) => (
                        <Badge key={`route-${agent.agentId}-${route}`} tone="neutral">
                          <RouteIcon className="mr-1 size-3" />
                          {route}
                        </Badge>
                      ))}
                      {agent.tools?.map((tool) => (
                        <Badge key={`tool-${agent.agentId}-${tool}`} tone="neutral">
                          <KeyRoundIcon className="mr-1 size-3" />
                          {tool}
                        </Badge>
                      ))}
                    </div>
                  ) : null}
                </div>
              ))}
            </div>
          ) : (
            <p className="text-sm text-[var(--fg-5)]">No agents reported by this runtime.</p>
          )}
        </PagePanel>

        <PagePanel className="p-5">
          <div className="mb-4 flex items-center gap-2">
            <DatabaseIcon className="size-4 text-[var(--fg-4)]" />
            <h2 className="text-sm font-semibold text-[var(--fg-1)]">Runtime Summary</h2>
          </div>
          <JsonPane
            ariaLabel="Runtime summary"
            value={data.runtime ?? { workspaceKey, status: "not reported" }}
          />
        </PagePanel>
      </div>
    </PageShell>
  );
}
