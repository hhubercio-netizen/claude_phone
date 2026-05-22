export interface ActionKey {
  label: string;
  /** Bytes to send on tap. */
  bytes: number[];
  /** Optional long-press alternative. */
  longPressBytes?: number[];
}

export const DEFAULT_KEYS: ActionKey[] = [
  { label: 'Esc',  bytes: [0x1b] },
  { label: 'Tab',  bytes: [0x09] },
  { label: '↑',    bytes: [0x1b, 0x5b, 0x41] },
  { label: '↓',    bytes: [0x1b, 0x5b, 0x42] },
  { label: '←',    bytes: [0x1b, 0x5b, 0x44] },
  { label: '→',    bytes: [0x1b, 0x5b, 0x43] },
  { label: '↵',    bytes: [0x0d] },
  { label: '^C',   bytes: [0x03] },
  { label: '^D',   bytes: [0x04] },
  { label: '/',    bytes: [0x2f] },
];
