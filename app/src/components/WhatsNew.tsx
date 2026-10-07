import { RELEASES } from "../content/whatsNew";
import { forPlatform } from "../lib/platform";
import { Modal } from "./Modal";

interface Props {
  version: string;
  onClose: () => void;
}

export function WhatsNew({ version, onClose }: Props) {
  const base = version.split("-")[0];
  const release = RELEASES.find((r) => r.version === base) ?? RELEASES[0];
  const highlights = release.highlights.map(forPlatform).filter((line): line is string => line !== null);
  return (
    <Modal
      title={`What's new in AutoLogin ${base}`}
      onClose={onClose}
      footer={
        <button className="button primary" onClick={onClose}>
          Got it
        </button>
      }
    >
      <ul className="whats-new">
        {highlights.map((line) => (
          <li key={line}>{line}</li>
        ))}
      </ul>
    </Modal>
  );
}
