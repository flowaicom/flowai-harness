import { z } from "zod";

const assertNever = (value: never): never => {
  throw new Error(`Unexpected user action: ${String(value)}`);
};

export const UserActionType = {
  PROCEED_PLAN: "proceed_plan",
  CANCEL_PLAN: "cancel_plan",
  MODIFY_PLAN: "modify_plan",
  SELECT_OPTION: "select_option",
  CONFIRM: "confirm",
  REJECT: "reject",
} as const;

export type UserActionType = (typeof UserActionType)[keyof typeof UserActionType];

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

export const isUserActionMessage = (text: string): boolean => {
  try {
    const parsed = JSON.parse(text);
    return parsed?.type === "user_action";
  } catch {
    return false;
  }
};

export const parseUserAction = (text: string): UserAction | null => {
  try {
    const parsed = JSON.parse(text);
    const result = userActionSchema.safeParse(parsed);
    return result.success ? result.data : null;
  } catch {
    return null;
  }
};

export const createUserActionMessage = (
  action: UserActionType,
  options?: {
    readonly planId?: string;
    readonly optionId?: string;
    readonly metadata?: Record<string, unknown>;
  }
): string => {
  const payload: UserAction = {
    type: "user_action",
    action,
    ...(options?.planId ? { planId: options.planId } : {}),
    ...(options?.optionId ? { optionId: options.optionId } : {}),
    ...(options?.metadata ? { metadata: options.metadata } : {}),
  };

  return JSON.stringify(payload);
};

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
    default:
      return assertNever(action);
  }
};

export const getUserActionType = (text: string): UserActionType | null => {
  const action = parseUserAction(text);
  return action?.action ?? null;
};
