import {
  type ConnectDocumentLike,
  type ConnectKnowledgeItemLike,
  ConnectKnowledgePage,
  type ConnectKnowledgeRuntimeLike,
} from "@studio/features-connect";
import { useMemo, useState } from "react";
import { SourcePicker } from "~/components/connect/source-picker";
import { useSourceId } from "~/lib/hooks/use-source-id";
import { useHarnessRuntime } from "~/lib/runtime/harness-runtime-context";
import { useScramble } from "~/lib/scramble";
import {
  connectScopeKey,
  connectUnavailable,
  documentSummaryToConnectDocument,
  knowledgeItemToConnectKnowledgeItem,
  mapConnectResult,
  metricSummaryToConnectMetric,
} from "./connect-package-adapters";

export default function KnowledgePage() {
  const { s } = useScramble();
  const { adapter, scope } = useHarnessRuntime();
  const { sourceId, setSourceId } = useSourceId("target");
  const [documents, setDocuments] = useState<ConnectDocumentLike[]>([]);
  const [knowledgeItems, setKnowledgeItems] = useState<ConnectKnowledgeItemLike[]>([]);
  const scopeKey = connectScopeKey({ scope, sourceId });

  const runtime = useMemo<ConnectKnowledgeRuntimeLike>(
    () => ({
      async listDocuments(inputScope) {
        const result = await adapter.listDocuments(inputScope);
        return mapConnectResult(result, (items) => items.map(documentSummaryToConnectDocument));
      },
      async browseKnowledge(inputScope) {
        const result = await adapter.browseKnowledge(inputScope, { sourceId });
        return mapConnectResult(result, (items) => items.map(knowledgeItemToConnectKnowledgeItem));
      },
      async listMetrics(inputScope) {
        const result = await adapter.listMetrics(inputScope);
        return mapConnectResult(result, (items) => items.map(metricSummaryToConnectMetric));
      },
      async deleteDocument() {
        return connectUnavailable("Document deletion is not available in harness mode.");
      },
      async deleteKnowledgeItem() {
        return connectUnavailable("Knowledge item deletion is not available in harness mode.");
      },
      async deleteMetric() {
        return connectUnavailable("Metric deletion is not available in harness mode.");
      },
      async extractKnowledge() {
        return connectUnavailable("Document-level extraction is not available in harness mode.");
      },
      async ingestDocuments() {
        return connectUnavailable("Browser document upload is not available in harness mode.");
      },
      async ingestKnowledgeFromSource(inputScope, source, handlers) {
        return adapter.ingestKnowledge(inputScope, {
          source:
            source.type === "localDirectory"
              ? {
                  type: "localDirectory",
                  path: source.path,
                  extensions: ["md", "txt", "json"],
                }
              : source,
          extractKnowledge: true,
          handlers: {
            onEvent: (event) => {
              handlers.onEvent(event as Parameters<typeof handlers.onEvent>[0]);
            },
            onComplete: handlers.onComplete,
            onError: handlers.onError,
          },
        });
      },
    }),
    [adapter, sourceId]
  );

  return (
    <ConnectKnowledgePage
      scope={scope}
      scopeKey={scopeKey}
      runtime={runtime}
      documents={documents}
      knowledgeItems={knowledgeItems}
      setDocuments={setDocuments}
      setKnowledgeItems={setKnowledgeItems}
      addDocument={(document) => setDocuments((previous) => [...previous, document])}
      removeDocument={(id) =>
        setDocuments((previous) => previous.filter((document) => document.id !== id))
      }
      removeKnowledgeItem={(id) =>
        setKnowledgeItems((previous) => previous.filter((item) => item.id !== id))
      }
      uploadTargetId={null}
      reloadKey={sourceId ?? "target"}
      headerAccessory={<SourcePicker sourceId={sourceId} onSourceChange={setSourceId} />}
      subtitle="Inspect harness-ingested documents and knowledge items."
      formatText={s}
    />
  );
}
