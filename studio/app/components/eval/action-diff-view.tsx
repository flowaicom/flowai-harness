import { ChevronDownIcon, ChevronRightIcon } from "lucide-react";
import { useCallback, useState } from "react";
import type {
  ActionComparisonDetail,
  ActionMatchResult,
  ActionStatus,
  EnrichmentDiagnostics,
  ProductComparison,
  ScopeComparison,
} from "~/lib/domain/eval";
import { useScramble } from "~/lib/scramble";
import { cn } from "~/lib/utils";
import { ActionStatusPill } from "./action-status";

// =============================================================================
// Border accent colors per status (pipeline style)
// =============================================================================

const BORDER_COLORS: Record<ActionStatus, string> = {
  exact: "border-l-[var(--dot-emerald)]",
  products_wrong: "border-l-[var(--dot-amber)]",
  scope_wrong: "border-l-[var(--dot-orange)]",
  both_wrong: "border-l-[var(--dot-red)]",
  missing: "border-l-muted-foreground/40 border-dashed",
  extra: "border-l-[var(--dot-purple)]",
};

const BG_COLORS: Record<ActionStatus, string> = {
  exact: "",
  products_wrong: "bg-[var(--accent-amber)]",
  scope_wrong: "bg-[var(--accent-orange)]",
  both_wrong: "bg-[var(--accent-red)]",
  missing: "bg-muted/30",
  extra: "bg-[var(--accent-purple)]",
};

function defaultExpanded(status: ActionStatus): boolean {
  return status !== "exact";
}

// =============================================================================
// Signature parser — "price:increase:3.00" → labeled fields
// =============================================================================

function ParsedSignature({ signature }: { readonly signature: string }) {
  const parts = signature.split(":");
  if (parts.length < 2) {
    return <span className="font-mono text-xs">{signature}</span>;
  }
  return (
    <span className="font-mono text-xs flex items-center gap-1 flex-wrap">
      <span className="font-semibold">{parts[0]}</span>
      <span className="text-muted-foreground">:</span>
      <span>{parts[1]}</span>
      {parts.length > 2 && (
        <>
          <span className="text-muted-foreground">:</span>
          <span className="text-muted-foreground">{parts.slice(2).join(":")}</span>
        </>
      )}
    </span>
  );
}

// =============================================================================
// ProductDiff — count bars + Jaccard bar
// =============================================================================

function ProductDiff({
  products,
  status,
}: {
  readonly products: ProductComparison;
  readonly status: ActionStatus;
}) {
  if (status === "missing" || status === "extra") {
    const count = status === "missing" ? products.expectedCount : products.actualCount;
    const label = status === "missing" ? "Expected" : "Actual";
    return (
      <div className="text-xs space-y-1">
        <div className="text-muted-foreground font-medium">Products</div>
        <div className="flex items-center gap-2">
          <span>
            {label}: {count}
          </span>
        </div>
      </div>
    );
  }

  if (products.exactMatch) {
    return (
      <div className="text-xs space-y-1">
        <div className="text-muted-foreground font-medium">Products</div>
        <div className="flex items-center gap-2 text-[var(--dot-emerald)]">
          Match ({products.expectedCount})
        </div>
      </div>
    );
  }

  const onlyExpected = products.expectedCount - products.matchedCount;
  const onlyActual = products.actualCount - products.matchedCount;
  const jaccardPct = Math.round(products.jaccard * 100);

  return (
    <div className="text-xs space-y-1.5">
      <div className="text-muted-foreground font-medium">Products</div>
      <div className="flex items-center gap-3 flex-wrap">
        <span className="tabular-nums">
          <span className="text-[var(--dot-emerald)]">{products.matchedCount} shared</span>
          {onlyExpected > 0 && (
            <span className="text-[var(--dot-red)] ml-2">-{onlyExpected} missing</span>
          )}
          {onlyActual > 0 && (
            <span className="text-[var(--dot-purple)] ml-2">+{onlyActual} extra</span>
          )}
        </span>
      </div>
      {/* Jaccard bar */}
      <div className="flex items-center gap-2">
        <div className="flex-1 h-1.5 rounded-full bg-muted overflow-hidden max-w-[120px]">
          <div
            className="h-full rounded-full bg-[var(--dot-amber)] transition-all"
            style={{ width: `${jaccardPct}%` }}
          />
        </div>
        <span className="text-muted-foreground tabular-nums">J={products.jaccard.toFixed(2)}</span>
      </div>
    </div>
  );
}

// =============================================================================
// ScopeDiff — channel tag comparison
// =============================================================================

function ScopeDiff({
  scope,
  status,
}: {
  readonly scope: ScopeComparison;
  readonly status: ActionStatus;
}) {
  if (status === "missing") {
    return (
      <div className="text-xs space-y-1">
        <div className="text-muted-foreground font-medium">Channels</div>
        <div className="flex gap-1 flex-wrap">
          {scope.expectedChannels.map((ch) => (
            <ChannelTag key={ch} channel={ch} variant="expected" />
          ))}
        </div>
      </div>
    );
  }

  if (status === "extra") {
    return (
      <div className="text-xs space-y-1">
        <div className="text-muted-foreground font-medium">Channels</div>
        <div className="flex gap-1 flex-wrap">
          {scope.actualChannels.map((ch) => (
            <ChannelTag key={ch} channel={ch} variant="extra" />
          ))}
        </div>
      </div>
    );
  }

  if (scope.channelsMatch) {
    return (
      <div className="text-xs space-y-1">
        <div className="text-muted-foreground font-medium">Channels</div>
        <div className="text-[var(--dot-emerald)]">Match ({scope.expectedChannels.length})</div>
      </div>
    );
  }

  const expectedSet = new Set(scope.expectedChannels);
  const actualSet = new Set(scope.actualChannels);
  const shared = scope.expectedChannels.filter((ch) => actualSet.has(ch));
  const missing = scope.expectedChannels.filter((ch) => !actualSet.has(ch));
  const extra = scope.actualChannels.filter((ch) => !expectedSet.has(ch));

  return (
    <div className="text-xs space-y-1">
      <div className="text-muted-foreground font-medium">Channels</div>
      <div className="flex gap-1 flex-wrap">
        {shared.map((ch) => (
          <ChannelTag key={`s-${ch}`} channel={ch} variant="shared" />
        ))}
        {missing.map((ch) => (
          <ChannelTag key={`m-${ch}`} channel={ch} variant="missing" />
        ))}
        {extra.map((ch) => (
          <ChannelTag key={`e-${ch}`} channel={ch} variant="extra" />
        ))}
      </div>
    </div>
  );
}

function ChannelTag({
  channel,
  variant,
}: {
  readonly channel: string;
  readonly variant: "shared" | "missing" | "extra" | "expected";
}) {
  const styles: Record<typeof variant, string> = {
    shared: "bg-[var(--accent-emerald)] text-[var(--dot-emerald)] border-[var(--dot-emerald)]/30",
    missing: "bg-[var(--accent-red)] text-[var(--dot-red)] line-through border-[var(--dot-red)]/30",
    extra: "bg-[var(--accent-purple)] text-[var(--dot-purple)] border-[var(--dot-purple)]/30",
    expected: "bg-muted text-muted-foreground border-border",
  };

  const prefix: Record<typeof variant, string> = {
    shared: "",
    missing: "\u2212",
    extra: "+",
    expected: "",
  };

  const { s } = useScramble();

  return (
    <span
      className={cn(
        "inline-flex items-center rounded px-1.5 py-0.5 text-[10px] font-mono border",
        styles[variant]
      )}
    >
      {prefix[variant]}
      {s(channel)}
    </span>
  );
}

// =============================================================================
// ActionDiffCard — expandable card per action comparison
// =============================================================================

function ActionDiffCard({
  detail,
  forceExpanded,
}: {
  readonly detail: ActionComparisonDetail;
  readonly forceExpanded: boolean | null;
}) {
  const [localOpen, setLocalOpen] = useState(defaultExpanded(detail.status));
  const isOpen = forceExpanded !== null ? forceExpanded : localOpen;
  const hasProducts = detail.products !== undefined;
  const hasScope = detail.scope !== undefined;
  const hasExpandedContent = hasProducts || hasScope;

  return (
    <div
      className={cn(
        "border border-l-2 rounded-md overflow-hidden transition-colors",
        BORDER_COLORS[detail.status],
        BG_COLORS[detail.status]
      )}
    >
      {/* Header (always visible) */}
      <button
        type="button"
        className="w-full flex items-center gap-2 px-3 py-2 text-left hover:bg-muted/40 transition-colors focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none"
        onClick={() => setLocalOpen((o) => !o)}
      >
        {hasExpandedContent && isOpen ? (
          <ChevronDownIcon className="size-3.5 text-muted-foreground shrink-0" />
        ) : (
          <ChevronRightIcon className="size-3.5 text-muted-foreground shrink-0" />
        )}
        <ActionStatusPill status={detail.status} />
        <ParsedSignature signature={detail.signature} />
        {detail.status !== "missing" && detail.status !== "extra" && detail.products && (
          <span className="text-[10px] text-muted-foreground font-mono ml-auto shrink-0 tabular-nums flex items-center gap-1.5">
            J={detail.products.jaccard.toFixed(2)}
            {detail.productEvidence && detail.productEvidence !== "uuids" && (
              <span
                className={cn(
                  "px-1 rounded text-[9px]",
                  detail.productEvidence === "fingerprints"
                    ? "bg-[var(--accent-emerald)] text-[var(--dot-emerald)]"
                    : "bg-muted text-muted-foreground/60"
                )}
              >
                {detail.productEvidence === "fingerprints" ? "fp" : "vac"}
              </span>
            )}
          </span>
        )}
      </button>

      {/* Body (expanded) */}
      {isOpen && hasExpandedContent && (
        <div className="px-3 pb-3 pt-1 space-y-2 border-t border-border/50">
          {detail.products && <ProductDiff products={detail.products} status={detail.status} />}
          {detail.scope && <ScopeDiff scope={detail.scope} status={detail.status} />}
        </div>
      )}
    </div>
  );
}

// =============================================================================
// ActionDiffView — toolbar + list of diff cards + issues
// =============================================================================

export function ActionDiffView({ actionMatch }: { readonly actionMatch: ActionMatchResult }) {
  const { actions, issues } = actionMatch;
  // null = respect per-card default, true/false = override all
  const [forceExpanded, setForceExpanded] = useState<boolean | null>(null);

  const expandAll = useCallback(() => setForceExpanded(true), []);
  const collapseAll = useCallback(() => setForceExpanded(false), []);
  const resetOverride = useCallback(() => setForceExpanded(null), []);

  if (actions.length === 0) return null;

  return (
    <div className="space-y-2">
      {/* Fallback indicator */}
      {actionMatch.totalSetFallback && (
        <div className="rounded-md accent-bar-blue bg-[var(--accent-blue)] px-3 py-1.5 text-xs text-[var(--dot-blue)]">
          Passed via total product-set fallback (per-action products differ but unions match)
        </div>
      )}

      {/* Toolbar */}
      <div className="flex items-center gap-2 text-[11px]">
        <span className="text-muted-foreground">{actions.length} actions</span>
        <span className="text-muted-foreground/40">|</span>
        <button
          type="button"
          onClick={expandAll}
          className="text-muted-foreground hover:text-foreground transition-colors rounded focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none"
        >
          Expand all
        </button>
        <button
          type="button"
          onClick={collapseAll}
          className="text-muted-foreground hover:text-foreground transition-colors rounded focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none"
        >
          Collapse all
        </button>
        {forceExpanded !== null && (
          <button
            type="button"
            onClick={resetOverride}
            className="text-[var(--dot-blue)] hover:text-[var(--dot-blue)]/80 transition-colors rounded focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none"
          >
            Reset
          </button>
        )}
      </div>

      {/* Diff cards — pipeline-like vertical list */}
      <div className="space-y-1.5">
        {actions.map((detail, index) => (
          <ActionDiffCard
            key={`${detail.index ?? index}-${detail.status}-${detail.signature}`}
            detail={detail}
            forceExpanded={forceExpanded}
          />
        ))}
      </div>

      {/* Issues list */}
      {issues.length > 0 && (
        <div className="rounded-md accent-bar-amber bg-[var(--accent-amber)] px-3 py-2 space-y-1">
          <div className="text-xs font-medium text-[var(--dot-amber)]">Issues</div>
          <ul className="text-xs text-muted-foreground list-disc list-inside space-y-0.5">
            {issues.map((issue) => (
              <li key={issue}>{issue}</li>
            ))}
          </ul>
        </div>
      )}

      {/* Enrichment diagnostics */}
      {actionMatch.enrichmentDiagnostics && (
        <EnrichmentDiagnosticsPanel diagnostics={actionMatch.enrichmentDiagnostics} />
      )}
    </div>
  );
}

// =============================================================================
// Enrichment Diagnostics — observability into KV enrichment
// =============================================================================

function EnrichmentDiagnosticsPanel({
  diagnostics,
}: {
  readonly diagnostics: EnrichmentDiagnostics;
}) {
  const checks = [
    { label: "Plan", found: diagnostics.planFound },
    { label: "Products", found: diagnostics.productSetFound },
    { label: "Scope", found: diagnostics.scopeSetFound },
  ];

  return (
    <div className="rounded-md border border-border/60 bg-muted/20 px-3 py-2 space-y-1.5">
      <div className="section-label">Enrichment</div>
      <div className="flex items-center gap-3 text-xs">
        {checks.map(({ label, found }) => (
          <span key={label} className="flex items-center gap-1">
            <span className={found ? "text-[var(--dot-emerald)]" : "text-[var(--dot-red)]"}>
              {found ? "\u2713" : "\u2717"}
            </span>
            {label}
          </span>
        ))}
        <span className="text-muted-foreground/40">|</span>
        <span className="text-muted-foreground tabular-nums">
          {diagnostics.actionCount} actions, {diagnostics.productCount} products,{" "}
          {diagnostics.fingerprintCount > 0 && <>{diagnostics.fingerprintCount} fingerprints, </>}
          {diagnostics.channelCount} channels
        </span>
      </div>
      {diagnostics.notes.length > 0 && (
        <ul className="text-[11px] text-muted-foreground space-y-0.5 list-disc list-inside">
          {diagnostics.notes.map((note) => (
            <li key={note}>{note}</li>
          ))}
        </ul>
      )}
    </div>
  );
}
