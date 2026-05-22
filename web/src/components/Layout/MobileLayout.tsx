import React from 'react';
import { useVisualViewport } from '../../hooks/useVisualViewport';

interface Props {
  header?: React.ReactNode;
  body: React.ReactNode;
  footer?: React.ReactNode;
}

export function MobileLayout({ header, body, footer }: Props) {
  const vv = useVisualViewport();
  return (
    <div
      className="flex flex-col w-full"
      style={{ height: vv.height }}
    >
      {header && <div className="flex-none">{header}</div>}
      <div className="flex-1 min-h-0 overflow-hidden">{body}</div>
      {footer && <div className="flex-none">{footer}</div>}
    </div>
  );
}
