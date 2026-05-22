import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { render, screen } from '@testing-library/react';
import { ErrorBoundary } from '../src/components/ErrorBoundary/ErrorBoundary';

function Boom({ msg }: { msg: string }): JSX.Element {
  throw new Error(msg);
}

let errSpy: ReturnType<typeof vi.spyOn>;

beforeEach(() => {
  errSpy = vi.spyOn(console, 'error').mockImplementation(() => {});
});

afterEach(() => {
  errSpy.mockRestore();
});

describe('ErrorBoundary', () => {
  it('renders children when no error', () => {
    render(
      <ErrorBoundary>
        <div data-testid="child">hi</div>
      </ErrorBoundary>
    );
    expect(screen.getByTestId('child')).toBeInTheDocument();
  });

  it('renders ErrorPage fallback when child throws', () => {
    render(
      <ErrorBoundary>
        <Boom msg="something bad happened" />
      </ErrorBoundary>
    );
    expect(screen.getByText(/Something went wrong/i)).toBeInTheDocument();
    expect(screen.getByText(/something bad happened/i)).toBeInTheDocument();
  });

  it('calls console.error when catching', () => {
    render(
      <ErrorBoundary>
        <Boom msg="boom" />
      </ErrorBoundary>
    );
    expect(errSpy).toHaveBeenCalled();
  });
});
