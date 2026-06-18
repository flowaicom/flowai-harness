/**
 * Tests index — shown when no test case is selected.
 *
 * Empty state with stats summary when test cases exist.
 *
 * @module routes/tests/_index
 */

import { ClipboardCheckIcon, FlaskConicalIcon, MessageSquareIcon, PlusIcon } from "lucide-react";
import { useNavigate } from "react-router";
import { StudioActionButton, StudioActionLink } from "~/components/shared/studio-action";
import { selectTestCases, useTestSuite } from "~/lib/stores";

export default function TestsIndex() {
  const navigate = useNavigate();
  const testCases = useTestSuite(selectTestCases);

  const hasTestCases = testCases.length > 0;

  return (
    <div className="flex-1 flex items-center justify-center text-muted-foreground">
      <div className="text-center max-w-sm">
        <div className="w-16 h-16 mx-auto mb-6 rounded-2xl bg-muted/50 flex items-center justify-center">
          <ClipboardCheckIcon className="size-8 text-muted-foreground/30" />
        </div>

        <p className="text-base font-medium text-foreground mb-1">
          {hasTestCases ? "Select a test case" : "No test cases yet"}
        </p>
        <p className="text-sm mb-6">
          {hasTestCases
            ? "Choose from the sidebar or create a new one"
            : "Create from scratch, or save a chat trace as a test case"}
        </p>

        <div className="flex flex-wrap items-center justify-center gap-2">
          <StudioActionButton onClick={() => navigate("/tests/new")} icon={PlusIcon} tone="strong">
            New Test Case
          </StudioActionButton>
          {!hasTestCases && (
            <StudioActionLink to="/chat" icon={MessageSquareIcon}>
              Create from Chat
            </StudioActionLink>
          )}
          {hasTestCases && (
            <StudioActionButton
              onClick={() => {
                const ids = testCases.map((tc) => tc.id);
                navigate(`/evals/new?testCaseIds=${encodeURIComponent(ids.join(","))}`);
              }}
              icon={FlaskConicalIcon}
            >
              Run Eval
            </StudioActionButton>
          )}
        </div>
      </div>
    </div>
  );
}
