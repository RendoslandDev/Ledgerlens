import { HIGH_CONFIDENCE, stateLabels, type QueueState } from "../data";

export function StatusDot({ state }: { state: QueueState }) {
  return (
    <span className={`status-dot ${state}`} title={stateLabels[state]}>
      <span className="visually-hidden">{stateLabels[state]}</span>
    </span>
  );
}

export function ConfidenceMeter({ value }: { value: number }) {
  const tone =
    value === 0 ? "none" : value >= HIGH_CONFIDENCE ? "high" : value >= 70 ? "mid" : "low";
  return (
    <span className={`confidence-meter ${tone}`} title={`${value}% confidence`}>
      <span className="confidence-track">
        <i style={{ width: `${value}%` }} />
      </span>
      <span className="mono">{value ? value : "—"}</span>
    </span>
  );
}
