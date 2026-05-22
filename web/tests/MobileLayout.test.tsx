import { describe, it, expect } from 'vitest';
import { render, screen } from '@testing-library/react';
import { MobileLayout } from '../src/components/Layout/MobileLayout';

describe('MobileLayout', () => {
  it('renders body content', () => {
    render(<MobileLayout body={<div data-testid="b">body-text</div>} />);
    expect(screen.getByTestId('b')).toHaveTextContent('body-text');
  });

  it('renders optional header and footer', () => {
    render(
      <MobileLayout
        header={<div data-testid="h">head</div>}
        body={<div>body</div>}
        footer={<div data-testid="f">foot</div>}
      />
    );
    expect(screen.getByTestId('h')).toBeInTheDocument();
    expect(screen.getByTestId('f')).toBeInTheDocument();
  });

  it('omits header and footer wrappers when not provided', () => {
    const { container } = render(
      <MobileLayout body={<div data-testid="b">only body</div>} />
    );
    expect(screen.getByTestId('b')).toBeInTheDocument();
    // Top-level flex container + body wrapper = 2 children expected.
    const root = container.firstElementChild!;
    expect(root.children.length).toBe(1);
  });
});
