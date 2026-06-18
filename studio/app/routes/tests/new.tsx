/**
 * New test case page — uses the same rich TestCaseForm as the detail page
 * (trajectory tool palette + ground-truth editor), saved via createTestCase.
 *
 * The LLM builder-chat is deferred (no harness builder-session backend); this
 * manual form + "Save as test case" from the Playground are the create paths.
 *
 * Supports `?prefill=<text>` to pre-populate the input prompt.
 *
 * @module routes/tests/new
 */

import { useCallback, useState } from "react";
import { useNavigate, useSearchParams } from "react-router";
import { ErrorBanner } from "~/components/shared/error-banner";
import { TestCaseForm } from "~/components/tests/test-case-form";
import { createTestCase } from "~/lib/api";
import type { TrajectoryMode } from "~/lib/domain/eval";
import { isOk } from "~/lib/domain/result";
import type { GroundTruth } from "~/lib/domain/test-case";
import { parseStructuredGroundTruthJson } from "~/lib/domain/test-case";
import { selectAvailableTools, useTestSuite, useTestSuiteActions } from "~/lib/stores";

export default function NewTestCase() {
  const [searchParams] = useSearchParams();
  const navigate = useNavigate();
  const availableTools = useTestSuite(selectAvailableTools);
  const { addTestCase: addToStore } = useTestSuiteActions();

  const [name, setName] = useState("");
  const [description, setDescription] = useState("");
  const [input, setInput] = useState(searchParams.get("prefill") ?? "");
  const [trajectory, setTrajectory] = useState("");
  const [mode, setMode] = useState<TrajectoryMode>("unordered");
  const [structuredGroundTruthJson, setStructuredGroundTruthJson] = useState("");
  const [tags, setTags] = useState("");
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const handleCreate = useCallback(async () => {
    setError(null);
    if (!input.trim()) {
      setError("Input prompt is required.");
      return;
    }

    const expectedTrajectory = trajectory
      .split(/\r?\n/)
      .map((s) => s.trim())
      .filter(Boolean);

    if (availableTools.length > 0) {
      const known = new Set(availableTools.map((t) => t.name));
      const unknown = expectedTrajectory.filter((t) => !known.has(t));
      if (unknown.length > 0) {
        setError(`Unknown tools in trajectory: ${unknown.join(", ")}`);
        return;
      }
    }

    let structuredGroundTruth: GroundTruth | null = null;
    try {
      structuredGroundTruth = parseStructuredGroundTruthJson(structuredGroundTruthJson);
    } catch (e) {
      setError(e instanceof Error ? e.message : "Structured ground truth must be valid JSON.");
      return;
    }

    setSaving(true);
    const result = await createTestCase({
      name: name.trim() || input.slice(0, 80).trim() || "Untitled",
      description: description.trim() || null,
      input: input.trim(),
      status: "draft",
      expectedTrajectory,
      trajectoryMode: mode,
      groundTruth: null,
      structuredGroundTruth,
      tags: tags
        .split(",")
        .map((s) => s.trim())
        .filter(Boolean),
      trajectoryProvenance: [],
      trajectorySources: [],
      sourceThreadId: null,
      sourceSessionId: null,
    });
    setSaving(false);

    if (isOk(result)) {
      addToStore(result.value);
      navigate(`/tests/${result.value.id}`);
    } else {
      setError(result.error.message);
    }
  }, [
    name,
    description,
    input,
    trajectory,
    mode,
    structuredGroundTruthJson,
    tags,
    availableTools,
    addToStore,
    navigate,
  ]);

  return (
    <div className="flex-1 overflow-y-auto">
      <div className="mx-auto w-full max-w-2xl space-y-5 p-6">
        <div className="flex items-center justify-between">
          <h1 className="text-sm font-semibold">New Test Case</h1>
          <div className="flex gap-2">
            <button
              type="button"
              onClick={() => navigate("/tests")}
              className="rounded-md border px-3 py-1.5 text-sm hover:bg-muted"
            >
              Cancel
            </button>
            <button
              type="button"
              onClick={handleCreate}
              disabled={saving}
              className="rounded-md bg-primary px-3 py-1.5 text-sm font-medium text-primary-foreground hover:bg-primary/90 disabled:opacity-50"
            >
              {saving ? "Creating…" : "Create Test Case"}
            </button>
          </div>
        </div>

        {error && <ErrorBanner message={error} onDismiss={() => setError(null)} />}

        <TestCaseForm
          name={name}
          onNameChange={setName}
          description={description}
          onDescriptionChange={setDescription}
          input={input}
          onInputChange={setInput}
          trajectory={trajectory}
          onTrajectoryChange={setTrajectory}
          mode={mode}
          onModeChange={setMode}
          structuredGroundTruthJson={structuredGroundTruthJson}
          onStructuredGroundTruthJsonChange={setStructuredGroundTruthJson}
          tags={tags}
          onTagsChange={setTags}
          availableTools={availableTools}
        />
      </div>
    </div>
  );
}
