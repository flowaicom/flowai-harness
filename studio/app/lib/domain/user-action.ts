/**
 * User Action Protocol for type-safe UI interactions.
 *
 * Structured JSON format for user actions that:
 * - Is WAF-safe (no XSS-triggering patterns)
 * - Is type-safe via Zod validation
 * - Is extensible for new action types
 *
 * @module domain/user-action
 */

import { z } from "zod";

// ============================================================================
// Action Types
// ============================================================================

export const UserActionType = {
  PROCEED_PLAN: "proceed_plan",
  CANCEL_PLAN: "cancel_plan",
  MODIFY_PLAN: "modify_plan",
  SELECT_OPTION: "select_option",
  CONFIRM: "confirm",
  REJECT: "reject",
} as const;

export type UserActionType = (typeof UserActionType)[keyof typeof UserActionType];

// ============================================================================
// Schema
// ============================================================================

export const userActionSchema = z.object({
  type: z.literal("user_action"),
  action: z.enum([
    UserActionType.PROCEED_PLAN,
    UserActionType.CANCEL_PLAN,
    UserActionType.MODIFY_PLAN,
    UserActionType.SELECT_OPTION,
    UserActionType.CONFIRM,
    UserActionType.REJECT,
  ]),
  planId: z.string().optional(),
  optionId: z.string().optional(),
  metadata: z.record(z.string(), z.unknown()).optional(),
});

export type UserAction = z.infer<typeof userActionSchema>;

// ============================================================================
// Type Guards
// ============================================================================

export const isUserActionMessage = (text: string): boolean => {
  try {
    const parsed = JSON.parse(text);
    return parsed?.type === "user_action";
  } catch {
    return false;
  }
};

// ============================================================================
// Parsers
// ============================================================================

export const parseUserAction = (text: string): UserAction | null => {
  try {
    const parsed = JSON.parse(text);
    const result = userActionSchema.safeParse(parsed);
    return result.success ? result.data : null;
  } catch {
    return null;
  }
};

// ============================================================================
// Constructors
// ============================================================================

export const createUserActionMessage = (
  action: UserActionType,
  options?: {
    planId?: string;
    optionId?: string;
    metadata?: Record<string, unknown>;
  }
): string => {
  const payload: UserAction = {
    type: "user_action",
    action,
    ...(options?.planId && { planId: options.planId }),
    ...(options?.optionId && { optionId: options.optionId }),
    ...(options?.metadata && { metadata: options.metadata }),
  };
  return JSON.stringify(payload);
};

// ============================================================================
// Display Helpers
// ============================================================================

export const getUserActionLabel = (action: UserActionType): string => {
  switch (action) {
    case UserActionType.PROCEED_PLAN:
      return "Proceed with Plan";
    case UserActionType.CANCEL_PLAN:
      return "Cancel Plan";
    case UserActionType.MODIFY_PLAN:
      return "Modify Plan";
    case UserActionType.SELECT_OPTION:
      return "Select Option";
    case UserActionType.CONFIRM:
      return "Confirm";
    case UserActionType.REJECT:
      return "Reject";
  }
};

export const getUserActionType = (text: string): UserActionType | null => {
  const action = parseUserAction(text);
  return action?.action ?? null;
};
