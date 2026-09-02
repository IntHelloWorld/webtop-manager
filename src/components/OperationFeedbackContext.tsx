import { createContext, useCallback, useContext, useMemo, useRef, useState, type ReactNode } from "react";

export type OperationKind =
  | "environmentCreate"
  | "environmentDelete"
  | "environmentStart"
  | "environmentStop"
  | "environmentRestart"
  | "publish"
  | "unpublish"
  | "imageDelete"
  | "cachePrune";

export interface ActiveOperation {
  id: string;
  kind: OperationKind;
  target: string;
}

interface OperationFeedbackValue {
  activeOperation: ActiveOperation | null;
  beginOperation: (kind: OperationKind, target: string) => string | null;
  finishOperation: (id: string) => void;
}

const OperationFeedbackContext = createContext<OperationFeedbackValue | null>(null);

export function OperationFeedbackProvider({ children }: { children: ReactNode }) {
  const [activeOperation, setActiveOperation] = useState<ActiveOperation | null>(null);
  const activeRef = useRef<ActiveOperation | null>(null);
  const sequence = useRef(0);

  const beginOperation = useCallback((kind: OperationKind, target: string) => {
    if (activeRef.current) return null;
    sequence.current += 1;
    const operation = { id: `${Date.now()}-${sequence.current}`, kind, target };
    activeRef.current = operation;
    setActiveOperation(operation);
    return operation.id;
  }, []);
  const finishOperation = useCallback((id: string) => {
    if (activeRef.current?.id !== id) return;
    activeRef.current = null;
    setActiveOperation(null);
  }, []);
  const value = useMemo(() => ({ activeOperation, beginOperation, finishOperation }), [activeOperation, beginOperation, finishOperation]);

  return <OperationFeedbackContext.Provider value={value}>{children}</OperationFeedbackContext.Provider>;
}

export function useOperationFeedback(): OperationFeedbackValue {
  const value = useContext(OperationFeedbackContext);
  if (!value) throw new Error("useOperationFeedback must be used inside OperationFeedbackProvider");
  return value;
}
