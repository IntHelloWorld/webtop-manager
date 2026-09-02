import { createPortal } from "react-dom";
import { useEffect, useId, useLayoutEffect, useRef, useState } from "react";

interface HelpTipProps {
  label: string;
  text: string;
}

export function HelpTip({ label, text }: HelpTipProps) {
  const tooltipId = useId();
  const triggerRef = useRef<HTMLSpanElement | null>(null);
  const tooltipRef = useRef<HTMLSpanElement | null>(null);
  const [open, setOpen] = useState(false);
  const [position, setPosition] = useState<{ left: number; top: number; maxWidth: number } | null>(null);

  const updatePosition = () => {
    const trigger = triggerRef.current;
    if (!trigger) return;
    const rect = trigger.getBoundingClientRect();
    const viewportPadding = 10;
    const gap = 8;
    const maxWidth = Math.max(180, Math.min(300, window.innerWidth - viewportPadding * 2));
    const tooltipWidth = Math.min(tooltipRef.current?.offsetWidth ?? maxWidth, maxWidth);
    const tooltipHeight = tooltipRef.current?.offsetHeight ?? 64;
    const left = Math.min(
      Math.max(viewportPadding, rect.left + rect.width / 2 - tooltipWidth / 2),
      window.innerWidth - tooltipWidth - viewportPadding,
    );
    const placeBelow = rect.top < tooltipHeight + gap + viewportPadding;
    const top = placeBelow
      ? Math.min(window.innerHeight - tooltipHeight - viewportPadding, rect.bottom + gap)
      : Math.max(viewportPadding, rect.top - tooltipHeight - gap);
    setPosition({ left, top, maxWidth });
  };

  useLayoutEffect(() => {
    if (!open) return;
    updatePosition();
    const frame = requestAnimationFrame(updatePosition);
    return () => cancelAnimationFrame(frame);
  }, [open, text]);

  useEffect(() => {
    if (!open) return;
    const reposition = () => updatePosition();
    window.addEventListener("resize", reposition);
    window.addEventListener("scroll", reposition, true);
    return () => {
      window.removeEventListener("resize", reposition);
      window.removeEventListener("scroll", reposition, true);
    };
  }, [open]);

  return (
    <span className="help-tip">
      <span
        ref={triggerRef}
        className="help-trigger"
        role="button"
        tabIndex={0}
        aria-label={`${label}: ${text}`}
        aria-describedby={open ? tooltipId : undefined}
        onMouseEnter={() => setOpen(true)}
        onMouseLeave={() => setOpen(false)}
        onFocus={() => setOpen(true)}
        onBlur={() => setOpen(false)}
        onKeyDown={(event) => { if (event.key === "Escape") setOpen(false); }}
      >
        ?
      </span>
      {open && position ? createPortal(
        <span
          ref={tooltipRef}
          className="help-tooltip"
          id={tooltipId}
          role="tooltip"
          style={{ left: position.left, top: position.top, maxWidth: position.maxWidth }}
        >
          {text}
        </span>,
        document.body,
      ) : null}
    </span>
  );
}
