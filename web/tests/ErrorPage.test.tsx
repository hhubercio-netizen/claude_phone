import { describe, it, expect } from 'vitest';
import { render, screen } from '@testing-library/react';
import { ErrorPage } from '../src/pages/ErrorPage';

describe('ErrorPage', () => {
  it('shows the error message when given an Error instance', () => {
    render(<ErrorPage error={new Error('disk on fire')} />);
    expect(screen.getByText(/Something went wrong/i)).toBeInTheDocument();
    expect(screen.getByText(/disk on fire/i)).toBeInTheDocument();
  });

  it('stringifies non-Error values', () => {
    render(<ErrorPage error={'plain string'} />);
    expect(screen.getByText(/plain string/i)).toBeInTheDocument();
  });

  it('renders a Reload button', () => {
    render(<ErrorPage error={new Error('x')} />);
    expect(screen.getByRole('button', { name: /reload/i })).toBeInTheDocument();
  });
});
