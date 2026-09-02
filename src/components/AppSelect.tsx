import { createPortal } from "react-dom";
import { forwardRef, useEffect, useId, useLayoutEffect, useRef, useState } from "react";

export interface AppSelectOption {
  value: string;
  label: string;
  group?: string;
  disabled?: boolean;
}

interface AppSelectProps {
  value: string;
  options: AppSelectOption[];
  onChange: (value: string) => void;
  onBlur?: () => void;
  name?: string;
  disabled?: boolean;
  ariaLabel?: string;
  ariaInvalid?: boolean;
  className?: string;
}

interface MenuPosition {
  left: number;
  top: number;
  width: number;
  maxHeight: number;
}

export const AppSelect = forwardRef<HTMLButtonElement, AppSelectProps>(function AppSelect({
  value,
  options,
  onChange,
  onBlur,
  name,
  disabled = false,
  ariaLabel,
  ariaInvalid = false,
  className = "",
}, forwardedRef) {
  const listboxId = useId();
  const triggerRef = useRef<HTMLButtonElement | null>(null);
  const menuRef = useRef<HTMLDivElement | null>(null);
  const [open, setOpen] = useState(false);
  const [activeIndex, setActiveIndex] = useState(() => Math.max(0, options.findIndex((option) => option.value === value)));
  const [position, setPosition] = useState<MenuPosition | null>(null);
  const selected = options.find((option) => option.value === value) ?? options[0];

  const setTriggerRef = (node: HTMLButtonElement | null) => {
    triggerRef.current = node;
    if (typeof forwardedRef === "function") forwardedRef(node);
    else if (forwardedRef) forwardedRef.current = node;
  };

  const updatePosition = () => {
    const trigger = triggerRef.current;
    if (!trigger) return;
    const rect = trigger.getBoundingClientRect();
    const viewportPadding = 8;
    const gap = 6;
    const width = Math.min(Math.max(rect.width, 180), window.innerWidth - viewportPadding * 2);
    const left = Math.min(Math.max(viewportPadding, rect.left), window.innerWidth - width - viewportPadding);
    const spaceBelow = window.innerHeight - rect.bottom - gap - viewportPadding;
    const spaceAbove = rect.top - gap - viewportPadding;
    const estimatedHeight = Math.min(320, options.length * 38 + 16);
    const placeAbove = spaceBelow < Math.min(180, estimatedHeight) && spaceAbove > spaceBelow;
    const maxHeight = Math.max(96, Math.min(320, placeAbove ? spaceAbove : spaceBelow));
    const renderedHeight = Math.min(menuRef.current?.scrollHeight ?? estimatedHeight, maxHeight);
    const top = placeAbove
      ? Math.max(viewportPadding, rect.top - renderedHeight - gap)
      : Math.min(window.innerHeight - renderedHeight - viewportPadding, rect.bottom + gap);
    setPosition({ left, top, width, maxHeight });
  };

  useLayoutEffect(() => {
    if (!open) return;
    updatePosition();
    const frame = requestAnimationFrame(updatePosition);
    return () => cancelAnimationFrame(frame);
  }, [open, options.length]);

  useEffect(() => {
    if (!open) return;
    const selectedIndex = options.findIndex((option) => option.value === value);
    setActiveIndex(Math.max(0, selectedIndex));
    const closeOnOutsidePointer = (event: PointerEvent) => {
      const target = event.target as Node;
      if (!triggerRef.current?.contains(target) && !menuRef.current?.contains(target)) setOpen(false);
    };
    const reposition = () => updatePosition();
    document.addEventListener("pointerdown", closeOnOutsidePointer, true);
    window.addEventListener("resize", reposition);
    window.addEventListener("scroll", reposition, true);
    return () => {
      document.removeEventListener("pointerdown", closeOnOutsidePointer, true);
      window.removeEventListener("resize", reposition);
      window.removeEventListener("scroll", reposition, true);
    };
  }, [open, options, value]);

  const choose = (option: AppSelectOption) => {
    if (option.disabled) return;
    onChange(option.value);
    setOpen(false);
    triggerRef.current?.focus();
  };

  const moveActive = (direction: 1 | -1) => {
    if (options.length === 0) return;
    let next = activeIndex;
    do next = (next + direction + options.length) % options.length;
    while (options[next]?.disabled && next !== activeIndex);
    setActiveIndex(next);
  };

  const groups = options.reduce<Array<{ label?: string; options: Array<{ option: AppSelectOption; index: number }> }>>((result, option, index) => {
    const last = result.at(-1);
    if (!last || last.label !== option.group) result.push({ label: option.group, options: [] });
    result.at(-1)?.options.push({ option, index });
    return result;
  }, []);

  return (
    <span className={`app-select ${className}`.trim()}>
      {name ? <input type="hidden" name={name} value={value} /> : null}
      <button
        ref={setTriggerRef}
        type="button"
        className="app-select-trigger"
        role="combobox"
        aria-label={ariaLabel}
        aria-controls={listboxId}
        aria-activedescendant={open && options[activeIndex] ? `${listboxId}-option-${activeIndex}` : undefined}
        aria-expanded={open}
        aria-haspopup="listbox"
        aria-invalid={ariaInvalid || undefined}
        disabled={disabled}
        onBlur={onBlur}
        onClick={() => setOpen((current) => !current)}
        onKeyDown={(event) => {
          if (event.key === "ArrowDown" || event.key === "ArrowUp") {
            event.preventDefault();
            if (!open) setOpen(true);
            else moveActive(event.key === "ArrowDown" ? 1 : -1);
          } else if (event.key === "Enter" || event.key === " ") {
            event.preventDefault();
            if (!open) setOpen(true);
            else if (options[activeIndex]) choose(options[activeIndex]);
          } else if (event.key === "Escape" && open) {
            event.preventDefault();
            setOpen(false);
          } else if (event.key === "Home" && open) {
            event.preventDefault();
            setActiveIndex(0);
          } else if (event.key === "End" && open) {
            event.preventDefault();
            setActiveIndex(Math.max(0, options.length - 1));
          }
        }}
      >
        <span>{selected?.label ?? value}</span>
        <span className="app-select-chevron" aria-hidden="true" />
      </button>
      {open && position ? createPortal(
        <div
          ref={menuRef}
          id={listboxId}
          className="app-select-menu"
          role="listbox"
          aria-label={ariaLabel}
          style={{ left: position.left, top: position.top, width: position.width, maxHeight: position.maxHeight }}
        >
          {groups.map((group, groupIndex) => (
            <div className="app-select-group" role="group" aria-label={group.label} key={`${group.label ?? "default"}-${groupIndex}`}>
              {group.label ? <div className="app-select-group-label">{group.label}</div> : null}
              {group.options.map(({ option, index }) => (
                <div
                  id={`${listboxId}-option-${index}`}
                  className={`app-select-option${option.value === value ? " selected" : ""}${index === activeIndex ? " active" : ""}`}
                  role="option"
                  aria-selected={option.value === value}
                  aria-disabled={option.disabled || undefined}
                  key={option.value}
                  onMouseEnter={() => setActiveIndex(index)}
                  onMouseDown={(event) => event.preventDefault()}
                  onClick={() => choose(option)}
                >
                  <span>{option.label}</span>
                  {option.value === value ? <span aria-hidden="true">✓</span> : null}
                </div>
              ))}
            </div>
          ))}
        </div>,
        document.body,
      ) : null}
    </span>
  );
});
