import { useEffect, useRef } from "react";

interface OperationConsoleProps {
  label: string;
  status: string;
  lines?: readonly string[];
  emptyMessage: string;
}

export function OperationConsole({ label, status, lines, emptyMessage }: OperationConsoleProps) {
  const outputRef = useRef<HTMLPreElement>(null);

  useEffect(() => {
    const output = outputRef.current;
    if (output) output.scrollTop = output.scrollHeight;
  }, [lines]);

  return <section className="operation-console" aria-label={label}>
    <header><strong>{label}</strong><span>{status}</span></header>
    <pre ref={outputRef} tabIndex={0} role="log" aria-live="polite" aria-relevant="additions text">{lines?.length ? lines.join("\n") : emptyMessage}</pre>
  </section>;
}
