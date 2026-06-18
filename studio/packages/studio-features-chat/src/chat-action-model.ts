import {
  type UserActionType as ChatUserActionType,
  createUserActionMessage,
  UserActionType,
} from "@studio/core/domain/user-action";

export interface CommandCardActionContextLike {
  readonly planId?: string;
}

export interface LinkedTestActions {
  readonly count: number;
  readonly testsHref: string;
  readonly testsLabel: string;
  readonly testsTitle: string;
  readonly evalHref: string;
  readonly evalTitle: string;
}

export interface ChatMarkdownPartLike {
  readonly type: string;
  readonly text?: string;
  readonly toolName?: string;
  readonly result?: unknown;
}

export interface ChatMarkdownMessageLike {
  readonly role: string;
  readonly parts: readonly ChatMarkdownPartLike[];
}

const USER_ACTION_TYPES = new Set<string>(Object.values(UserActionType));

export function isSupportedUserActionType(actionId: string): actionId is ChatUserActionType {
  return USER_ACTION_TYPES.has(actionId);
}

export function createCommandCardActionMessage(
  actionId: string,
  context: CommandCardActionContextLike
): string | null {
  if (!isSupportedUserActionType(actionId)) {
    return null;
  }

  return createUserActionMessage(actionId, {
    planId: context.planId,
  });
}

export function prependLinkedTestId(linkedTestIds: readonly string[], nextId: string): string[] {
  return [nextId, ...linkedTestIds.filter((id) => id !== nextId)];
}

export function getLinkedTestActions(linkedTestIds: readonly string[]): LinkedTestActions | null {
  if (linkedTestIds.length === 0) {
    return null;
  }

  const count = linkedTestIds.length;
  const pluralSuffix = count > 1 ? "s" : "";

  return {
    count,
    testsHref: count === 1 ? `/tests/${linkedTestIds[0]}` : "/tests",
    testsLabel: `${count} Test${pluralSuffix}`,
    testsTitle: `${count} test case${pluralSuffix} from this thread`,
    evalHref: `/evals/new?testCaseIds=${encodeURIComponent(linkedTestIds.join(","))}`,
    evalTitle: "Run evaluation on all test cases from this thread",
  };
}

export function buildChatMarkdown(messages: readonly ChatMarkdownMessageLike[]): string {
  const lines: string[] = [];

  for (const message of messages) {
    const role = message.role === "user" ? "User" : "Assistant";
    lines.push(`## ${role}\n`);

    for (const part of message.parts) {
      if (part.type === "text" && typeof part.text === "string") {
        lines.push(part.text);
        continue;
      }

      if (part.type === "tool-invocation" && typeof part.toolName === "string") {
        lines.push(`> **Tool:** ${part.toolName}`);
        if (part.result !== undefined) {
          lines.push(`> ${String(part.result)}`);
        }
      }
    }

    lines.push("");
  }

  return lines.join("\n");
}
