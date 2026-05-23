import { describe, it, expect, beforeEach } from 'vitest';
import { renderHook, act } from '@testing-library/react';
import { useFontSize } from '../src/hooks/useFontSize';

beforeEach(() => {
  localStorage.clear();
});

describe('useFontSize', () => {
  it('starts at the default size when nothing is stored', () => {
    const { result } = renderHook(() => useFontSize());
    expect(result.current.size).toBe(result.current.default);
  });

  it('reads a previously stored size on mount', () => {
    localStorage.setItem('cp.fontSize', '17');
    const { result } = renderHook(() => useFontSize());
    expect(result.current.size).toBe(17);
  });

  it('clamps a stored value below MIN', () => {
    localStorage.setItem('cp.fontSize', '4');
    const { result } = renderHook(() => useFontSize());
    expect(result.current.size).toBe(result.current.min);
  });

  it('clamps a stored value above MAX', () => {
    localStorage.setItem('cp.fontSize', '99');
    const { result } = renderHook(() => useFontSize());
    expect(result.current.size).toBe(result.current.max);
  });

  it('falls back to default when stored value is not a number', () => {
    localStorage.setItem('cp.fontSize', 'abc');
    const { result } = renderHook(() => useFontSize());
    expect(result.current.size).toBe(result.current.default);
  });

  it('inc/dec walk by 1 within bounds', () => {
    const { result } = renderHook(() => useFontSize());
    const start = result.current.size;
    act(() => result.current.inc());
    expect(result.current.size).toBe(start + 1);
    act(() => result.current.dec());
    expect(result.current.size).toBe(start);
  });

  it('inc clamps at MAX', () => {
    const { result } = renderHook(() => useFontSize());
    const max = result.current.max;
    for (let i = 0; i < max + 5; i++) act(() => result.current.inc());
    expect(result.current.size).toBe(max);
  });

  it('dec clamps at MIN', () => {
    const { result } = renderHook(() => useFontSize());
    const min = result.current.min;
    for (let i = 0; i < 50; i++) act(() => result.current.dec());
    expect(result.current.size).toBe(min);
  });

  it('persists size to localStorage on change', () => {
    const { result } = renderHook(() => useFontSize());
    act(() => result.current.inc());
    expect(localStorage.getItem('cp.fontSize')).toBe(String(result.current.size));
  });

  it('reset returns to default', () => {
    const { result } = renderHook(() => useFontSize());
    act(() => result.current.inc());
    act(() => result.current.reset());
    expect(result.current.size).toBe(result.current.default);
  });
});
