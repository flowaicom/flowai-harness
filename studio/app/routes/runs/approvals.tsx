import { useCallback, useEffect, useState } from "react";
import { ApprovalCard } from "~/components/approvals/approval-card";
import { type ApprovalRef, listApprovals } from "~/lib/api/approvals";
import { isOk } from "~/lib/domain/result";

/** Approval inbox — pending-first, with approve / reject / revise actions. */
export default function ApprovalsInbox() {
  const [approvals, setApprovals] = useState<readonly ApprovalRef[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);

  const load = useCallback(() => {
    listApprovals().then((result) => {
      if (isOk(result)) {
        setApprovals(result.value);
        setError(null);
      } else {
        setError(result.error.message);
      }
      setLoading(false);
    });
  }, []);

  useEffect(() => {
    load();
  }, [load]);

  if (loading) {
    return <p className="p-6 text-sm text-muted-foreground">Loading approvals…</p>;
  }
  if (error) {
    return <p className="p-6 text-sm text-red-600">{error}</p>;
  }
  if (approvals.length === 0) {
    return <p className="p-6 text-sm text-muted-foreground">No approvals captured yet.</p>;
  }

  return (
    <div className="space-y-3 p-6">
      {approvals.map((approval) => (
        <ApprovalCard key={approval.approvalId} approval={approval} onResolved={load} />
      ))}
    </div>
  );
}
