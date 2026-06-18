import type { z } from "zod";

import type { Result } from "~/lib/domain/result";
import { err, ok } from "~/lib/domain/result";
import type { ApiError } from "./client";
import { ResponseValidationError, validateResponse } from "./schemas";

export function validationErrorToApiError(context: string, error: unknown): ApiError {
  if (error instanceof ResponseValidationError) {
    return {
      code: "VALIDATION_ERROR",
      message: error.message,
      details: error.issues,
    };
  }

  return {
    code: "VALIDATION_ERROR",
    message: `${context} validation failed`,
    details: error,
  };
}

export function validateBoundary<T>(
  schema: z.ZodTypeAny,
  data: unknown,
  context: string
): Result<T, ApiError> {
  try {
    return ok(validateResponse(schema as z.ZodType<T>, data, context));
  } catch (error) {
    return err(validationErrorToApiError(context, error));
  }
}
