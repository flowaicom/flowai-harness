import { SharedTestCaseForm } from "@studio/features-tests";
import type { TrajectoryMode } from "~/lib/domain/eval";
import type { ToolCatalogEntry } from "~/lib/domain/test-case";

export interface TestCaseFormProps {
  name: string;
  onNameChange: (value: string) => void;
  /** When true the name is the immutable test-case id (detail view). */
  nameReadOnly?: boolean;
  description: string;
  onDescriptionChange: (value: string) => void;
  input: string;
  onInputChange: (value: string) => void;
  trajectory: string;
  onTrajectoryChange: (value: string) => void;
  mode: TrajectoryMode;
  onModeChange: (value: TrajectoryMode) => void;
  structuredGroundTruthJson: string;
  onStructuredGroundTruthJsonChange: (value: string) => void;
  tags: string;
  onTagsChange: (value: string) => void;
  availableTools: ToolCatalogEntry[];
}

function StructuredGroundTruthJsonEditor({
  value,
  onChange,
}: {
  readonly value: string;
  readonly onChange: (value: string) => void;
}) {
  return (
    <textarea
      id="structured-ground-truth"
      value={value}
      onChange={(event) => onChange(event.target.value)}
      placeholder='{ "executedActions": [ ... ], "payloadMatch": "exact" }'
      className="form-textarea min-h-24 w-full resize-y font-mono text-sm"
    />
  );
}

export function TestCaseForm({
  name,
  onNameChange,
  nameReadOnly = false,
  description,
  onDescriptionChange,
  input,
  onInputChange,
  trajectory,
  onTrajectoryChange,
  mode,
  onModeChange,
  structuredGroundTruthJson,
  onStructuredGroundTruthJsonChange,
  tags,
  onTagsChange,
  availableTools,
}: TestCaseFormProps) {
  return (
    <SharedTestCaseForm
      identity={{
        name,
        onNameChange,
        nameReadOnly,
        description,
        onDescriptionChange,
      }}
      input={input}
      onInputChange={onInputChange}
      trajectory={trajectory}
      onTrajectoryChange={onTrajectoryChange}
      mode={mode}
      onModeChange={onModeChange}
      tags={tags}
      onTagsChange={onTagsChange}
      status="draft"
      onStatusChange={() => {}}
      showStatus={false}
      availableTools={availableTools}
      toolCatalogUnavailableMessage="No tool catalog for this workspace - add expected tool names manually."
      groundTruthEditor={
        <StructuredGroundTruthJsonEditor
          value={structuredGroundTruthJson}
          onChange={onStructuredGroundTruthJsonChange}
        />
      }
    />
  );
}
