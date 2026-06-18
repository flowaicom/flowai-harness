import type { ApiError, ApiErrorCode, ApiResult } from "../domain/errors";
import { makeApiError } from "../domain/errors";
import { err, isOk, ok, type Result } from "../domain/result";

export type { ApiError, ApiErrorCode } from "../domain/errors";
export type ApiDecoder<T> = (input: unknown) => Result<T, ApiError>;
export interface ApiRequestOptions extends RequestInit {
  readonly timeoutMs?: number;
}

/**
 * API configuration.
 */
export interface ApiConfig {
  readonly baseUrl: string;
  readonly timeout: number;
  readonly headers: Record<string, string>;
}

const defaultConfig: ApiConfig = {
  baseUrl: "/api",
  timeout: 30000,
  headers: {
    "Content-Type": "application/json",
  },
};

let config: ApiConfig = defaultConfig;

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

export const setApiConfig = (newConfig: Partial<ApiConfig>): void => {
  config = { ...config, ...newConfig };
};

export const setWorkspaceHeader = (workspaceId: string): void => {
  const headers = { ...config.headers };
  if (workspaceId && workspaceId !== "default") {
    headers["X-Workspace-Id"] = workspaceId;
  } else {
    delete headers["X-Workspace-Id"];
  }
  config = { ...config, headers };
};

export const getApiConfig = (): ApiConfig => config;

export const apiFetch = async <T = unknown>(
  path: string,
  options: ApiRequestOptions = {}
): Promise<ApiResult<T>> => {
  const url = `${config.baseUrl}${path}`;
  const { timeoutMs, signal, headers, ...requestInit } = options;

  const controller = new AbortController();
  const timeoutId = setTimeout(() => controller.abort(), timeoutMs ?? config.timeout);
  const combinedSignal = signal ? AbortSignal.any([signal, controller.signal]) : controller.signal;

  try {
    const response = await fetch(url, {
      ...requestInit,
      headers: {
        ...config.headers,
        ...headers,
      },
      signal: combinedSignal,
    });

    clearTimeout(timeoutId);

    if (!response.ok) {
      const errorBody = await response.text().catch(() => "");
      const parsedError = parseErrorBody(errorBody);
      return err(
        makeApiError({
          code: statusToErrorCode(response.status),
          message: parsedError.message || response.statusText,
          status: response.status,
          details: parsedError.details,
        })
      );
    }

    const text = await response.text();
    if (!text) {
      return ok(undefined as T);
    }

    try {
      const data = JSON.parse(text) as unknown;
      if (data === null || typeof data !== "object") {
        return err(
          makeApiError({
            code: "VALIDATION_ERROR",
            message: "Unexpected response shape",
          })
        );
      }
      return ok(data as T);
    } catch {
      return ok(text as T);
    }
  } catch (error) {
    clearTimeout(timeoutId);

    if (error instanceof DOMException && error.name === "AbortError") {
      return err(
        makeApiError({
          code: "TIMEOUT",
          message: "Request timed out",
        })
      );
    }

    return err(
      makeApiError({
        code: "NETWORK_ERROR",
        message: error instanceof Error ? error.message : "Network error",
      })
    );
  }
};

export const get = <T = unknown>(
  path: string,
  options?: ApiRequestOptions
): Promise<ApiResult<T>> => apiFetch<T>(path, { ...options, method: "GET" });

export const getDecoded = async <T>(
  path: string,
  decode: ApiDecoder<T>,
  options?: ApiRequestOptions
): Promise<Result<T, ApiError>> => {
  const result = await apiFetch(path, { ...options, method: "GET" });
  return isOk(result) ? decode(result.value) : result;
};

export const post = <T = unknown>(
  path: string,
  body?: unknown,
  options?: ApiRequestOptions
): Promise<ApiResult<T>> =>
  apiFetch<T>(path, {
    ...options,
    method: "POST",
    body: body ? JSON.stringify(body) : undefined,
  });

export const postDecoded = async <T>(
  path: string,
  body: unknown,
  decode: ApiDecoder<T>,
  options?: ApiRequestOptions
): Promise<Result<T, ApiError>> => {
  const result = await apiFetch(path, {
    ...options,
    method: "POST",
    body: body ? JSON.stringify(body) : undefined,
  });
  return isOk(result) ? decode(result.value) : result;
};

export const put = <T = unknown>(
  path: string,
  body?: unknown,
  options?: ApiRequestOptions
): Promise<ApiResult<T>> =>
  apiFetch<T>(path, {
    ...options,
    method: "PUT",
    body: body ? JSON.stringify(body) : undefined,
  });

export const putDecoded = async <T>(
  path: string,
  body: unknown,
  decode: ApiDecoder<T>,
  options?: ApiRequestOptions
): Promise<Result<T, ApiError>> => {
  const result = await apiFetch(path, {
    ...options,
    method: "PUT",
    body: body ? JSON.stringify(body) : undefined,
  });
  return isOk(result) ? decode(result.value) : result;
};

export const patch = <T = unknown>(
  path: string,
  body?: unknown,
  options?: ApiRequestOptions
): Promise<ApiResult<T>> =>
  apiFetch<T>(path, {
    ...options,
    method: "PATCH",
    body: body ? JSON.stringify(body) : undefined,
  });

export const patchDecoded = async <T>(
  path: string,
  body: unknown,
  decode: ApiDecoder<T>,
  options?: ApiRequestOptions
): Promise<Result<T, ApiError>> => {
  const result = await apiFetch(path, {
    ...options,
    method: "PATCH",
    body: body ? JSON.stringify(body) : undefined,
  });
  return isOk(result) ? decode(result.value) : result;
};

export const del = <T = unknown>(
  path: string,
  options?: ApiRequestOptions
): Promise<ApiResult<T>> => apiFetch<T>(path, { ...options, method: "DELETE" });

export const delDecoded = async <T>(
  path: string,
  decode: ApiDecoder<T>,
  options?: ApiRequestOptions
): Promise<Result<T, ApiError>> => {
  const result = await apiFetch(path, { ...options, method: "DELETE" });
  return isOk(result) ? decode(result.value) : result;
};
