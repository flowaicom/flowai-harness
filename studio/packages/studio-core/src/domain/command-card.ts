import { z } from "zod";
import { err, ok, type Result } from "./result";

export interface CommandCardActionContext {
  readonly planId?: string;
  readonly dsl: string;
}

export type CommandCardActionHandler = (
  actionId: string,
  context: CommandCardActionContext
) => void | Promise<void>;

export type CommandCardDecision = "approved" | "cancelled" | "modification_requested";

export interface CommandCardAttribute {
  readonly label: string;
  readonly value: string;
  readonly explanation?: string;
  readonly section: "metrics" | "default" | "context";
  readonly collapsible: boolean;
  readonly defaultExpanded: boolean;
  readonly cardStyle?: string;
}

export interface CommandCardAction {
  readonly id: string;
  readonly label: string;
  readonly variant: "primary" | "secondary" | "danger";
  readonly disabled: boolean;
}

export interface CommandCardViewModel {
  readonly planId?: string;
  readonly title: string;
  readonly description?: string;
  readonly attributes: readonly CommandCardAttribute[];
  readonly actions: readonly CommandCardAction[];
}

export interface CommandCardParseError {
  readonly kind: "invalid-json" | "invalid-dsl";
  readonly message: string;
  readonly issues: readonly string[];
  readonly rawDsl: string;
}

const commandCardAttributeSchema = z
  .object({
    label: z.string().min(1),
    value: z.string(),
    explanation: z.string().optional(),
    section: z.string().optional(),
    collapsible: z.boolean().optional(),
    defaultExpanded: z.boolean().optional(),
    cardStyle: z.string().optional(),
  })
  .catchall(z.unknown());

const commandCardActionSchema = z
  .object({
    id: z.string().min(1),
    label: z.string().min(1),
    variant: z.enum(["primary", "secondary", "danger"]).optional(),
    disabled: z.boolean().optional(),
  })
  .catchall(z.unknown());

const commandCardPropsSchema = z
  .object({
    planId: z.string().optional(),
    title: z.string().min(1),
    description: z.string().optional(),
    attributes: z.array(commandCardAttributeSchema).optional(),
    actions: z.array(commandCardActionSchema).optional(),
  })
  .catchall(z.unknown());

const commandCardDslSchema = z
  .object({
    components: z.tuple([
      z
        .object({
          name: z.literal("CommandCard"),
          props: commandCardPropsSchema,
        })
        .catchall(z.unknown()),
    ]),
  })
  .catchall(z.unknown());

function normalizeSection(section?: string, cardStyle?: string): CommandCardAttribute["section"] {
  if (cardStyle === "stat-card") return "metrics";
  if (section === "metrics" || section === "context" || section === "default") return section;
  return "default";
}

export function normalizeCommandCardDsl(
  dsl: string
): Result<CommandCardViewModel, CommandCardParseError> {
  let parsed: unknown;

  try {
    parsed = JSON.parse(dsl);
  } catch (error) {
    return err({
      kind: "invalid-json",
      message: "Command card DSL is not valid JSON.",
      issues: [error instanceof Error ? error.message : "Unknown JSON parse error"],
      rawDsl: dsl,
    });
  }

  const validation = commandCardDslSchema.safeParse(parsed);
  if (!validation.success) {
    return err({
      kind: "invalid-dsl",
      message: "Command card DSL does not match the supported CommandCard structure.",
      issues: validation.error.issues.map((issue) => {
        const path = issue.path.length > 0 ? `${issue.path.join(".")}: ` : "";
        return `${path}${issue.message}`;
      }),
      rawDsl: dsl,
    });
  }

  const props = validation.data.components[0].props;
  return ok({
    planId: props.planId,
    title: props.title,
    description: props.description,
    attributes: (props.attributes ?? []).map((attribute) => ({
      label: attribute.label,
      value: attribute.value,
      explanation: attribute.explanation,
      section: normalizeSection(attribute.section, attribute.cardStyle),
      collapsible: attribute.collapsible ?? false,
      defaultExpanded: attribute.defaultExpanded ?? false,
      cardStyle: attribute.cardStyle,
    })),
    actions: (props.actions ?? []).map((action) => ({
      id: action.id,
      label: action.label,
      variant: action.variant ?? "secondary",
      disabled: action.disabled ?? false,
    })),
  });
}

export function formatCommandCardParseError(error: CommandCardParseError): string {
  if (error.issues.length === 0) return error.message;
  return [error.message, ...error.issues].join("\n");
}

export function decisionFromActionId(actionId: string): CommandCardDecision | null {
  switch (actionId) {
    case "proceed_plan":
      return "approved";
    case "cancel_plan":
      return "cancelled";
    case "modify_plan":
      return "modification_requested";
    default:
      return null;
  }
}
