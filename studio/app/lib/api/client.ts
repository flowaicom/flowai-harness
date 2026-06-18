/**
 * API client with Result-based error handling.
 *
 * All API functions return Result<T, ApiError> for explicit error handling.
 *
 * @module api/client
 */

import type { Result } from "~/lib/domain/result";
import { err, ok } from "~/lib/domain/result";

// ============================================================================
// Error Types
// ============================================================================

/**
 * API error with typed codes.
 */
export interface ApiError {
  readonly code: ApiErrorCode;
  readonly message: string;
  readonly status?: number;
  readonly details?: unknown;
}

/**
 * API error codes.
 */
export type ApiErrorCode =
  | "NETWORK_ERROR"
  | "TIMEOUT"
  | "UNAUTHORIZED"
  | "FORBIDDEN"
  | "NOT_FOUND"
  | "VALIDATION_ERROR"
  | "SERVER_ERROR"
  | "UNKNOWN";

/**
 * Map HTTP status to error code.
 */
const statusToErrorCode = (status: number): ApiErrorCode => {
  switch (status) {
    case 401:
      return "UNAUTHORIZED";
    case 403:
      return "FORBIDDEN";
    case 404:
      return "NOT_FOUND";
    case 422:
      return "VALIDATION_ERROR";
    default:
      return status >= 500 ? "SERVER_ERROR" : "UNKNOWN";
  }
};

const parseErrorBody = (body: string): Pick<ApiError, "message" | "details"> => {
  if (!body) {
    return { message: "" };
  }

  try {
    const parsed = JSON.parse(body) as {
      message?: unknown;
      error?: unknown;
      details?: unknown;
    };
    const message =
      typeof parsed.message === "string"
        ? parsed.message
        : typeof parsed.error === "string"
          ? parsed.error
          : body;
    return { message, details: parsed.details ?? parsed };
  } catch {
    return { message: body };
  }
};

// ============================================================================
// Configuration
// ============================================================================

/**
 * API configuration.
 */
export interface ApiConfig {
  readonly baseUrl: string;
  readonly timeout: number;
  readonly headers: Record<string, string>;
}

/**
 * Default configuration.
 */
const defaultConfig: ApiConfig = {
  baseUrl: "/api",
  timeout: 30000,
  headers: {
    "Content-Type": "application/json",
  },
};

let config: ApiConfig = defaultConfig;

/**
 * Set API configuration.
 */
export const setApiConfig = (newConfig: Partial<ApiConfig>): void => {
  config = { ...config, ...newConfig };
};

/**
 * Set the active workspace header on legacy API requests.
 *
 * Harness Studio routing is path-scoped via `/workspaces/:workspaceKey/...`.
 * This header remains for older routes and observability while feature screens
 * are cut over module-by-module.
 */
export const setWorkspaceHeader = (workspaceId: string): void => {
  const headers = { ...config.headers };
  headers["X-Workspace-Id"] = workspaceId || "default";
  config = { ...config, headers };
};

/**
 * Get current configuration.
 */
export const getApiConfig = (): ApiConfig => config;

// ============================================================================
// Fetch Wrapper
// ============================================================================

/**
 * Type-safe fetch wrapper with Result.
 */
export const apiFetch = async <T>(
  path: string,
  options: RequestInit = {}
): Promise<Result<T, ApiError>> => {
  const url = `${config.baseUrl}${path}`;

  const controller = new AbortController();
  const timeoutId = setTimeout(() => controller.abort(), config.timeout);

  try {
    const response = await fetch(url, {
      ...options,
      headers: {
        ...config.headers,
        ...options.headers,
      },
      signal: options.signal ?? controller.signal,
    });

    clearTimeout(timeoutId);

    if (!response.ok) {
      const errorBody = await response.text().catch(() => "");
      const parsedError = parseErrorBody(errorBody);
      return err({
        code: statusToErrorCode(response.status),
        message: parsedError.message || response.statusText,
        status: response.status,
        details: parsedError.details,
      });
    }

    // Handle empty responses
    const text = await response.text();
    if (!text) {
      return ok(undefined as T);
    }

    try {
      const data = JSON.parse(text) as T;
      return ok(data);
    } catch {
      // Return raw text if not JSON
      return ok(text as T);
    }
  } catch (error) {
    clearTimeout(timeoutId);

    if (error instanceof DOMException && error.name === "AbortError") {
      return err({
        code: "TIMEOUT",
        message: "Request timed out",
      });
    }

    return err({
      code: "NETWORK_ERROR",
      message: error instanceof Error ? error.message : "Network error",
    });
  }
};

// ============================================================================
// HTTP Methods
// ============================================================================

/**
 * GET request.
 */
export const get = <T>(path: string, options?: RequestInit): Promise<Result<T, ApiError>> =>
  apiFetch<T>(path, { ...options, method: "GET" });

/**
 * POST request.
 */
export const post = <T>(
  path: string,
  body?: unknown,
  options?: RequestInit
): Promise<Result<T, ApiError>> =>
  apiFetch<T>(path, {
    ...options,
    method: "POST",
    body: body ? JSON.stringify(body) : undefined,
  });

/**
 * PUT request.
 */
export const put = <T>(
  path: string,
  body?: unknown,
  options?: RequestInit
): Promise<Result<T, ApiError>> =>
  apiFetch<T>(path, {
    ...options,
    method: "PUT",
    body: body ? JSON.stringify(body) : undefined,
  });

/**
 * PATCH request.
 */
export const patch = <T>(
  path: string,
  body?: unknown,
  options?: RequestInit
): Promise<Result<T, ApiError>> =>
  apiFetch<T>(path, {
    ...options,
    method: "PATCH",
    body: body ? JSON.stringify(body) : undefined,
  });

/**
 * DELETE request.
 */
export const del = <T>(path: string, options?: RequestInit): Promise<Result<T, ApiError>> =>
  apiFetch<T>(path, { ...options, method: "DELETE" });
