import type { Result } from "./result";

export type ApiErrorCode =
  | "NETWORK_ERROR"
  | "TIMEOUT"
  | "UNAUTHORIZED"
  | "FORBIDDEN"
  | "NOT_FOUND"
  | "VALIDATION_ERROR"
  | "CAPABILITY_DISABLED"
  | "NOT_IMPLEMENTED"
  | "SERVER_ERROR"
  | "UNKNOWN";

export interface ApiError {
  readonly kind?: "api-error";
  readonly code: ApiErrorCode;
  readonly message: string;
  readonly status?: number;
  readonly details?: unknown;
}

export type ApiResult<T> = Result<T, ApiError>;

export type StreamErrorCode =
  | "STREAM_CONNECT_FAILED"
  | "STREAM_ABORTED"
  | "STREAM_PROTOCOL_ERROR"
  | "STREAM_DECODE_ERROR"
  | "STREAM_CLOSED"
  | "UNKNOWN";

export interface StreamError {
  readonly kind?: "stream-error";
  readonly code: StreamErrorCode;
  readonly message: string;
  readonly status?: number;
  readonly details?: unknown;
}

export type ParseErrorCode = "INVALID_JSON" | "INVALID_SCHEMA" | "UNSUPPORTED_SHAPE" | "UNKNOWN";

export interface ParseError {
  readonly kind?: "parse-error";
  readonly code: ParseErrorCode;
  readonly message: string;
  readonly issues: readonly string[];
  readonly rawInput?: string;
  readonly details?: unknown;
}

export function makeApiError(input: Omit<ApiError, "kind">): ApiError {
  return { kind: "api-error", ...input };
}

export function makeStreamError(input: Omit<StreamError, "kind">): StreamError {
  return { kind: "stream-error", ...input };
}

export function makeParseError(input: Omit<ParseError, "kind">): ParseError {
  return { kind: "parse-error", ...input };
}
