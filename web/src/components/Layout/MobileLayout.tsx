import React from 'react';

interface Props {
  header?: React.ReactNode;
  body: React.ReactNode;
  footer?: React.ReactNode;
}

// Height is driven by the html/body/#root rule in globals.css (100dvh +
// position: fixed) so the layout stays anchored to the visual viewport even
// while the mobile keyboard is animating in. The previous JS-driven
// visualViewport.height path raced with iOS Safari focus-into-view scroll
// behaviour and produced an empty black screen on first tap.
export function MobileLayout({ header, body, footer }: Props) {
  return (
    <div className="flex flex-col w-full h-full">
      {header && <div className="flex-none">{header}</div>}
      <div className="flex-1 min-h-0 overflow-hidden">{body}</div>
      {footer && <div className="flex-none">{footer}</div>}
    </div>
  );
}
