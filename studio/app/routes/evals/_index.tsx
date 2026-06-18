import { ClipboardCheckIcon, FlaskConicalIcon, PlusIcon } from "lucide-react";
import { useNavigate } from "react-router";
import { StudioActionButton, StudioActionLink } from "~/components/shared/studio-action";
import { selectEvalRuns, selectTestCaseCount, useEvaluation, useTestSuite } from "~/lib/stores";

/**
 * Evals index — shown when no eval run is selected.
 *
 * Lifecycle-aware empty state: when no test cases exist, guides user to
 * Tests tab first. Otherwise shows "New Eval" CTA.
 */
export default function EvalsIndex() {
  const navigate = useNavigate();
  const runs = useEvaluation(selectEvalRuns);
  const testCaseCount = useTestSuite(selectTestCaseCount);

  if (runs.length === 0) {
    return (
      <div className="flex-1 flex items-center justify-center text-muted-foreground">
        <div className="text-center max-w-sm">
          <div className="w-14 h-14 mx-auto mb-5 rounded-xl bg-muted/40 flex items-center justify-center">
            <FlaskConicalIcon className="w-7 h-7 opacity-25" />
          </div>
          <p className="text-sm font-medium text-foreground mb-1">No eval runs yet</p>
          <p className="text-sm mb-5">
            {testCaseCount > 0
              ? `You have ${testCaseCount} test case${testCaseCount > 1 ? "s" : ""} ready to evaluate`
              : "Create test cases first, then evaluate your agent against them"}
          </p>
          <div className="flex flex-wrap items-center justify-center gap-2">
            {testCaseCount > 0 ? (
              <StudioActionButton
                onClick={() => navigate("/evals/new")}
                icon={PlusIcon}
                tone="strong"
              >
                New Eval
              </StudioActionButton>
            ) : (
              <>
                <StudioActionLink to="/tests/new" icon={ClipboardCheckIcon} tone="strong">
                  Create Test Cases
                </StudioActionLink>
                <StudioActionLink to="/chat" icon={PlusIcon}>
                  Start from Chat
                </StudioActionLink>
              </>
            )}
          </div>
        </div>
      </div>
    );
  }

  return (
    <div className="flex-1 flex items-center justify-center text-muted-foreground">
      <div className="text-center max-w-sm">
        <div className="w-14 h-14 mx-auto mb-5 rounded-xl bg-muted/40 flex items-center justify-center">
          <FlaskConicalIcon className="w-7 h-7 opacity-25" />
        </div>
        <p className="text-sm font-medium text-foreground mb-1">Select an eval run</p>
        <p className="text-sm mb-5">Choose a run from the sidebar or create a new evaluation</p>
        <StudioActionButton onClick={() => navigate("/evals/new")} icon={PlusIcon} tone="strong">
          New Eval
        </StudioActionButton>
      </div>
    </div>
  );
}
