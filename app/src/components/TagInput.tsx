/** Longest name the app accepts (same limit as Cirrus's Account Tag). */
export const MAX_TAG_CHARS = 64;

const TAG_HINT = "Your label for this account, e.g. from Cirrus's Account Tag.";

interface Props {
  id: string;
  value: string;
  filledFromCirrus?: boolean;
  error: string | null;
  onChange: (value: string) => void;
}

/** The account's name field (the "tag"), shared by the account drawer and the bulk paste dialog. */
export function TagInput({ id, value, filledFromCirrus = false, error, onChange }: Props) {
  return (
    <div className="field">
      <label htmlFor={id}>
        Name<span className="muted"> (optional)</span>
      </label>
      <input
        id={id}
        className="input"
        type="text"
        autoComplete="off"
        maxLength={MAX_TAG_CHARS}
        value={value}
        aria-invalid={error ? true : undefined}
        aria-describedby={error ? `${id}-error` : `${id}-hint`}
        onChange={(e) => onChange(e.target.value)}
      />
      {filledFromCirrus && <span className="hint">Filled in from Cirrus.</span>}
      <span className="hint" id={`${id}-hint`}>
        {TAG_HINT}
      </span>
      {error && (
        <span className="error" id={`${id}-error`}>
          {error}
        </span>
      )}
    </div>
  );
}
