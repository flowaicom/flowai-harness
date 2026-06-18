import { type ConnectImportFileLike, ConnectImportPage } from "@studio/features-connect";
import { UploadIcon } from "lucide-react";
import { useState } from "react";
import { Link } from "react-router";

const EMPTY_PIPELINE_STAGES: readonly [] = [];

export default function ImportPage() {
  const [files, setFiles] = useState<ConnectImportFileLike[]>([]);

  return (
    <ConnectImportPage
      title="Import"
      subtitle="Generic CSV/parquet browser import is deferred until the harness owns a data.import API."
      fileUploadLabel="Import is not available in harness mode yet"
      files={files}
      onAddFiles={(nextFiles) =>
        setFiles((previous) => [
          ...previous,
          ...Array.from(nextFiles).map((file) => ({ name: file.name, size: file.size })),
        ])
      }
      onRemoveFile={(fileName) =>
        setFiles((previous) => previous.filter((file) => file.name !== fileName))
      }
      onStart={() => {}}
      onCancel={() => {}}
      onReset={() => setFiles([])}
      startDisabled={true}
      isRunning={false}
      isCompleted={false}
      isFailed={false}
      elapsedMs={0}
      pipelineStages={EMPTY_PIPELINE_STAGES}
      currentStage={null}
      profilingTotal={0}
      profilingCompleted={0}
      error={null}
      onDismissError={() => {}}
      showFileSection={false}
      showEmptyState={true}
      emptyState={{
        icon: UploadIcon,
        title: "Import is not available in harness mode yet",
        description:
          "Use the CLI or server-visible ingestion paths until a harness-native import/upload API exists.",
      }}
      nextSteps={
        <Link
          to="/connect"
          className="inline-flex items-center rounded-md border px-3 py-2 text-sm hover:bg-muted"
        >
          Back to Connect
        </Link>
      }
    />
  );
}
