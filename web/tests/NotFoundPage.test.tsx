import { describe, it, expect } from 'vitest';
import { render, screen } from '@testing-library/react';
import { NotFoundPage } from '../src/pages/NotFoundPage';

describe('NotFoundPage', () => {
  it('renders the not-found heading', () => {
    render(<NotFoundPage />);
    expect(screen.getByRole('heading', { name: /not found/i })).toBeInTheDocument();
  });

  it('mentions the /phone command in the body copy', () => {
    render(<NotFoundPage />);
    expect(screen.getByText(/\/phone/)).toBeInTheDocument();
  });
});
