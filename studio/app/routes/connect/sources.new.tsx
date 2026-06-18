import { DatabaseIcon } from "lucide-react";
import { Link } from "react-router";
import { EmptyState } from "~/components/shared/empty-state";
import { SectionCard, SectionHeader } from "~/components/shared/section-card";

export default function NewSourcePage() {
  return (
    <div className="flex-1 overflow-y-auto scroll-container">
      <div className="max-w-lg mx-auto p-6 space-y-6">
        <SectionCard>
          <SectionHeader>New Data Source</SectionHeader>
          <EmptyState
            icon={DatabaseIcon}
            title="Source CRUD is not available in harness mode"
            description="The local Studio reads the workspace data_environment from the Python/TS harness app. Add or change sources in runtime code, then restart the Studio dev server."
          />
          <div className="flex justify-center">
            <Link
              to="/connect"
              className="inline-flex items-center rounded-md border px-3 py-2 text-sm hover:bg-muted"
            >
              Back to Connect
            </Link>
          </div>
        </SectionCard>
      </div>
    </div>
  );
}
