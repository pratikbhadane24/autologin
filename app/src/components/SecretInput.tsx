import { useLayoutEffect, useRef, useState, type InputHTMLAttributes } from "react";
import "./SecretInput.css";

type InputProps = Omit<
  InputHTMLAttributes<HTMLInputElement>,
  "type" | "value" | "onChange" | "autoComplete" | "spellCheck"
>;

interface Props extends InputProps {
  id: string;
  /** The field's visible label, used to name the show/hide button. */
  label: string;
  value: string;
  onChange: (value: string) => void;
}

interface Selection {
  start: number | null;
  end: number | null;
}

/**
 * A secret input (password, PIN, TOTP secret...) with an eye button that shows
 * what the user is typing. It only ever holds what is typed now: saved secrets
 * never reach the UI. The value is hidden again when the field is cleared, and
 * when the dialog closes (the component unmounts).
 */
export function SecretInput({ id, label, value, onChange, className, ...rest }: Props) {
  const [revealed, setRevealed] = useState(false);
  const input = useRef<HTMLInputElement>(null);
  const restore = useRef<Selection | null>(null);

  // Cleared (by the user or a reset form): hide again before the next value.
  if (revealed && value === "") setRevealed(false);

  // After toggling, keep typing where the caret was.
  useLayoutEffect(() => {
    const selection = restore.current;
    if (!selection) return;
    restore.current = null;
    input.current?.focus();
    if (selection.start !== null && selection.end !== null) {
      input.current?.setSelectionRange(selection.start, selection.end);
    }
  }, [revealed]);

  function toggle() {
    restore.current = { start: input.current?.selectionStart ?? null, end: input.current?.selectionEnd ?? null };
    setRevealed((shown) => !shown);
  }

  return (
    <div className="secret-input">
      <input
        {...rest}
        ref={input}
        id={id}
        className={className ?? "input"}
        type={revealed ? "text" : "password"}
        autoComplete="off"
        autoCapitalize="off"
        autoCorrect="off"
        spellCheck={false}
        value={value}
        onChange={(e) => onChange(e.target.value)}
      />
      <button
        type="button"
        className="secret-toggle"
        aria-label={`${revealed ? "Hide" : "Show"} ${inSentence(label)}`}
        aria-pressed={revealed}
        aria-controls={id}
        // Keep focus (and the caret) in the input when clicked.
        onMouseDown={(e) => e.preventDefault()}
        onClick={toggle}
      >
        <EyeIcon crossed={revealed} />
      </button>
    </div>
  );
}

/** "Password" -> "password" for "Show password"; acronyms like "PIN" or "TOTP Secret" stay. */
export function inSentence(label: string): string {
  const [first = "", second = ""] = label;
  return second && second === second.toLowerCase() ? first.toLowerCase() + label.slice(1) : label;
}

function EyeIcon({ crossed }: { crossed: boolean }) {
  return (
    <svg
      width="18"
      height="18"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.8"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
      focusable="false"
    >
      <path d="M2 12s3.6-7 10-7 10 7 10 7-3.6 7-10 7S2 12 2 12Z" />
      <circle cx="12" cy="12" r="3" />
      {crossed && <path d="M4 4l16 16" />}
    </svg>
  );
}
