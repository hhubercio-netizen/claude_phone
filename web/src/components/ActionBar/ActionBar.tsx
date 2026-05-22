import { DEFAULT_KEYS, type ActionKey } from './keys';
import styles from './ActionBar.module.css';

interface Props {
  onKey: (bytes: Uint8Array) => void;
  keys?: ActionKey[];
}

export function ActionBar({ onKey, keys = DEFAULT_KEYS }: Props) {
  return (
    <div className={styles.bar}>
      {keys.map((k) => (
        <button
          key={k.label}
          className={styles.btn}
          onMouseDown={(e) => {
            e.preventDefault();
            onKey(new Uint8Array(k.bytes));
          }}
          onTouchStart={(e) => {
            e.preventDefault();
            onKey(new Uint8Array(k.bytes));
          }}
        >
          {k.label}
        </button>
      ))}
    </div>
  );
}
