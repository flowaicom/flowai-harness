import { Loader2Icon } from "lucide-react";
import { useEffect } from "react";
import { useNavigate } from "react-router";

export default function Index() {
  const navigate = useNavigate();

  useEffect(() => {
    navigate("/workspace", { replace: true });
  }, [navigate]);

  return (
    <div className="flex h-full items-center justify-center">
      <Loader2Icon className="size-6 animate-spin text-muted-foreground" />
    </div>
  );
}
