export interface FlowAIStudioConfig {
  readonly appName: string;
  readonly apiBaseUrl: string;
  readonly studioApiVersion: "harness-studio/v1" | string;
  readonly defaultWorkspaceKey: string;
  readonly streamTransport: "sse" | string;
}

export const DEFAULT_FLOWAI_STUDIO_CONFIG: FlowAIStudioConfig = {
  appName: "FlowAI Studio",
  apiBaseUrl: "/api",
  studioApiVersion: "harness-studio/v1",
  defaultWorkspaceKey: "default",
  streamTransport: "sse",
};

declare global {
  interface Window {
    readonly __FLOWAI__?: unknown;
  }
}

function asRecord(input: unknown): Record<string, unknown> {
  return input && typeof input === "object" && !Array.isArray(input)
    ? (input as Record<string, unknown>)
    : {};
}

function stringField(input: unknown, fallback: string): string {
  return typeof input === "string" && input.trim().length > 0 ? input : fallback;
}

export function normalizeFlowAIStudioConfig(
  input: unknown,
  fallback: FlowAIStudioConfig = DEFAULT_FLOWAI_STUDIO_CONFIG
): FlowAIStudioConfig {
  const value = asRecord(input);
  return {
    appName: stringField(value.appName, fallback.appName),
    apiBaseUrl: stringField(value.apiBaseUrl, fallback.apiBaseUrl),
    studioApiVersion: stringField(value.studioApiVersion, fallback.studioApiVersion),
    defaultWorkspaceKey: stringField(value.defaultWorkspaceKey, fallback.defaultWorkspaceKey),
    streamTransport: stringField(value.streamTransport, fallback.streamTransport),
  };
}

export function getFlowAIStudioConfig(): FlowAIStudioConfig {
  const value = typeof window === "undefined" ? undefined : window.__FLOWAI__;
  return normalizeFlowAIStudioConfig(value);
}
