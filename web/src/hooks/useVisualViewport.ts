import { useEffect, useState } from 'react';

export interface ViewportInfo {
  height: number;
  width: number;
  keyboardOpen: boolean;
}

export function useVisualViewport(): ViewportInfo {
  const [info, setInfo] = useState<ViewportInfo>(() => initial());

  useEffect(() => {
    const vv = window.visualViewport;
    if (!vv) return;
    const update = () => {
      const keyboardOpen = window.innerHeight - vv.height > 100;
      setInfo({ height: vv.height, width: vv.width, keyboardOpen });
    };
    update();
    vv.addEventListener('resize', update);
    vv.addEventListener('scroll', update);
    return () => {
      vv.removeEventListener('resize', update);
      vv.removeEventListener('scroll', update);
    };
  }, []);

  return info;
}

function initial(): ViewportInfo {
  const vv = window.visualViewport;
  return {
    height: vv?.height ?? window.innerHeight,
    width: vv?.width ?? window.innerWidth,
    keyboardOpen: false,
  };
}
