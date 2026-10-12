// Small controls shared by the deck panels. Deck buttons never take focus,
// so Space and the other deck keys keep working after a click.

import { useRef, useState, type ReactNode } from "react";

const noFocus = (e: React.MouseEvent) => e.preventDefault();

interface ButtonProps {
  className?: string;
  title?: string;
  disabled?: boolean;
  active?: boolean;
  children: ReactNode;
  onClick?: () => void;
  onDoubleClick?: () => void;
}

/** A deck button that acts on click. */
export function DeckButton({ className = "", title, disabled, active, children, onClick, onDoubleClick }: ButtonProps) {
  return (
    <button
      type="button"
      tabIndex={-1}
      className={`dbtn ${className}${active ? " on" : ""}`}
      title={title}
      disabled={disabled}
      onMouseDown={noFocus}
      onClick={onClick}
      onDoubleClick={onDoubleClick}
    >
      {children}
    </button>
  );
}

interface HoldProps extends Omit<ButtonProps, "onClick"> {
  onPress: () => void;
  onRelease?: () => void;
}

/** A button with press/release semantics (CUE, hot cues, pitch bend, tempo −/+). */
export function HoldButton({ className = "", title, disabled, active, children, onPress, onRelease }: HoldProps) {
  const [held, setHeld] = useState(false);
  return (
    <button
      type="button"
      tabIndex={-1}
      className={`dbtn ${className}${active ? " on" : ""}${held ? " held" : ""}`}
      title={title}
      disabled={disabled}
      onMouseDown={noFocus}
      onPointerDown={(e) => {
        if (e.button !== 0) return;
        e.currentTarget.setPointerCapture(e.pointerId);
        setHeld(true);
        onPress();
      }}
      onPointerUp={(e) => {
        if (!held) return;
        e.currentTarget.releasePointerCapture(e.pointerId);
        setHeld(false);
        onRelease?.();
      }}
      onPointerCancel={() => {
        if (!held) return;
        setHeld(false);
        onRelease?.();
      }}
    >
      {children}
    </button>
  );
}

interface DragValue {
  value: number;
  min: number;
  max: number;
  onChange: (value: number) => void;
  /** Double-click. */
  onReset?: () => void;
}

/**
 * Dragging shows its own value until release, so engine echoes of older
 * values never make a control jump.
 */
function useDrag(value: number) {
  const [dragValue, setDragValue] = useState<number | null>(null);
  return { shown: dragValue ?? value, dragValue, setDragValue };
}

interface KnobProps extends DragValue {
  label?: string;
  size?: number;
  /** Where the arc starts from: the minimum, or the centre for ±. */
  bipolar?: boolean;
  title?: string;
  className?: string;
}

const KNOB_SWEEP = 270;

/** A rotary knob: drag up/down (⇧ for fine), double-click to reset. */
export function Knob({ value, min, max, onChange, onReset, label, size = 34, bipolar, title, className = "" }: KnobProps) {
  const { shown, setDragValue } = useDrag(value);
  const start = useRef<{ y: number; value: number } | null>(null);
  const fraction = (Math.min(max, Math.max(min, shown)) - min) / (max - min);
  const angle = -KNOB_SWEEP / 2 + fraction * KNOB_SWEEP;
  const r = size / 2 - 3;
  const c = size / 2;
  const arc = (from: number, to: number) => {
    const a0 = ((from - 90) * Math.PI) / 180;
    const a1 = ((to - 90) * Math.PI) / 180;
    const large = Math.abs(to - from) > 180 ? 1 : 0;
    const sweep = to > from ? 1 : 0;
    return `M ${c + r * Math.cos(a0)} ${c + r * Math.sin(a0)} A ${r} ${r} 0 ${large} ${sweep} ${c + r * Math.cos(a1)} ${c + r * Math.sin(a1)}`;
  };
  const origin = bipolar ? 0 : -KNOB_SWEEP / 2;
  return (
    <div
      className={`knob ${className}`}
      title={title}
      style={{ width: size }}
      onPointerDown={(e) => {
        if (e.button !== 0) return;
        e.currentTarget.setPointerCapture(e.pointerId);
        start.current = { y: e.clientY, value: shown };
      }}
      onPointerMove={(e) => {
        if (!start.current) return;
        const range = max - min;
        const pixels = e.shiftKey ? 600 : 150;
        const next = Math.min(max, Math.max(min, start.current.value + ((start.current.y - e.clientY) / pixels) * range));
        setDragValue(next);
        onChange(next);
      }}
      onPointerUp={() => {
        start.current = null;
        setDragValue(null);
      }}
      onPointerCancel={() => {
        start.current = null;
        setDragValue(null);
      }}
      onDoubleClick={onReset}
    >
      <svg width={size} height={size} aria-hidden>
        <path d={arc(-KNOB_SWEEP / 2, KNOB_SWEEP / 2)} className="knob-track" />
        {Math.abs(angle - origin) > 0.5 && <path d={arc(Math.min(origin, angle), Math.max(origin, angle))} className="knob-value" />}
        <circle cx={c} cy={c} r={r - 4} className="knob-cap" />
        <line
          x1={c + (r - 12) * Math.sin((angle * Math.PI) / 180)}
          y1={c - (r - 12) * Math.cos((angle * Math.PI) / 180)}
          x2={c + (r - 4) * Math.sin((angle * Math.PI) / 180)}
          y2={c - (r - 4) * Math.cos((angle * Math.PI) / 180)}
          className="knob-pointer"
        />
      </svg>
      {label && <span className="knob-label">{label}</span>}
    </div>
  );
}

interface FaderProps extends DragValue {
  orientation?: "vertical" | "horizontal";
  /** Vertical faders put `max` at the top unless inverted (the crossfader has A on top). */
  inverted?: boolean;
  className?: string;
  title?: string;
  children?: ReactNode;
}

/** A fader: click or drag anywhere on it, double-click to reset. */
export function Fader({ value, min, max, onChange, onReset, orientation = "vertical", inverted, className = "", title, children }: FaderProps) {
  const { shown, setDragValue } = useDrag(value);
  const dragging = useRef(false);
  const vertical = orientation === "vertical";
  let fraction = (Math.min(max, Math.max(min, shown)) - min) / (max - min);
  if (vertical && !inverted) fraction = 1 - fraction;
  const valueAt = (e: React.PointerEvent<HTMLDivElement>) => {
    const rect = e.currentTarget.getBoundingClientRect();
    const thumb = 14;
    let f = vertical
      ? (e.clientY - rect.top - thumb / 2) / Math.max(1, rect.height - thumb)
      : (e.clientX - rect.left - thumb / 2) / Math.max(1, rect.width - thumb);
    f = Math.min(1, Math.max(0, f));
    if (vertical && !inverted) f = 1 - f;
    return min + f * (max - min);
  };
  return (
    <div
      className={`fader ${orientation} ${className}`}
      title={title}
      onPointerDown={(e) => {
        if (e.button !== 0) return;
        e.currentTarget.setPointerCapture(e.pointerId);
        dragging.current = true;
        const next = valueAt(e);
        setDragValue(next);
        onChange(next);
      }}
      onPointerMove={(e) => {
        if (!dragging.current) return;
        const next = valueAt(e);
        setDragValue(next);
        onChange(next);
      }}
      onPointerUp={() => {
        dragging.current = false;
        setDragValue(null);
      }}
      onPointerCancel={() => {
        dragging.current = false;
        setDragValue(null);
      }}
      onDoubleClick={onReset}
    >
      <div className="fader-track" />
      {children}
      <div
        className="fader-thumb"
        style={vertical ? { top: `calc(${fraction} * (100% - 14px))` } : { left: `calc(${fraction} * (100% - 14px))` }}
      />
    </div>
  );
}
