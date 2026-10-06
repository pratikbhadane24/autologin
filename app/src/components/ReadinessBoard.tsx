import type { Readiness, Segment } from "../lib/readiness";
import "./ReadinessBoard.css";

const SEGMENT_TEXT: Record<Segment, string> = {
  ready: "logged in",
  running: "logging in",
  failed: "failed",
  attention: "needs setup",
  waiting: "not logged in",
  unsupported: "broker coming soon",
};

interface Props {
  readiness: Readiness;
  nextRun: string | null;
}

/** The headline answer to "am I ready for market open?" plus one bar
 *  segment per account, coloured by state and updated live. */
export function ReadinessBoard({ readiness, nextRun }: Props) {
  const { headline, segments, ready, total } = readiness;
  return (
    <section className="readiness" aria-live="polite">
      <h1 className="readiness-headline">{headline}</h1>
      {total > 0 && (
        <ol className="readiness-bar" aria-label={`${ready} of ${total} accounts logged in`}>
          {segments.map((s) => (
            <li key={s.id} className={`segment segment-${s.state}`} title={`${s.label}: ${SEGMENT_TEXT[s.state]}`}>
              <span className="visually-hidden">
                {s.label}: {SEGMENT_TEXT[s.state]}
              </span>
            </li>
          ))}
        </ol>
      )}
      <p className="readiness-next muted">{nextRun ?? "Automatic daily login is off. Turn it on in Schedule."}</p>
    </section>
  );
}
