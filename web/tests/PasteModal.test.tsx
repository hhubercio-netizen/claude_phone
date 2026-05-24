import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, fireEvent } from '@testing-library/react';
import { PasteModal } from '../src/components/PasteModal/PasteModal';

beforeEach(() => {
  localStorage.clear();
  sessionStorage.clear();
});

describe('PasteModal', () => {
  it('renders nothing when closed', () => {
    const onClose = vi.fn();
    const onSend = vi.fn();
    render(<PasteModal open={false} onClose={onClose} onSend={onSend} />);
    expect(screen.queryByRole('dialog')).toBeNull();
  });

  it('renders dialog when open', () => {
    render(<PasteModal open onClose={vi.fn()} onSend={vi.fn()} />);
    expect(screen.getByRole('dialog')).toBeInTheDocument();
  });

  it('Send sends UTF-8 bytes of the textarea content and closes', () => {
    const onSend = vi.fn();
    const onClose = vi.fn();
    render(<PasteModal open onClose={onClose} onSend={onSend} />);
    const ta = screen.getByPlaceholderText(/Type or paste/i) as HTMLTextAreaElement;
    fireEvent.change(ta, { target: { value: 'hello żółć' } });
    fireEvent.click(screen.getByRole('button', { name: /^Send$/ }));
    expect(onSend).toHaveBeenCalledOnce();
    const bytes = onSend.mock.calls[0][0] as Uint8Array;
    const decoded = new TextDecoder().decode(bytes);
    expect(decoded).toBe('hello żółć');
    expect(onClose).toHaveBeenCalledOnce();
  });

  it('Send button is disabled when textarea is empty', () => {
    render(<PasteModal open onClose={vi.fn()} onSend={vi.fn()} />);
    const send = screen.getByRole('button', { name: /^Send$/ }) as HTMLButtonElement;
    expect(send.disabled).toBe(true);
  });

  it('Cancel closes without sending', () => {
    const onSend = vi.fn();
    const onClose = vi.fn();
    render(<PasteModal open onClose={onClose} onSend={onSend} />);
    fireEvent.click(screen.getByRole('button', { name: /Cancel/i }));
    expect(onSend).not.toHaveBeenCalled();
    expect(onClose).toHaveBeenCalled();
  });

  it('Escape closes the modal', () => {
    const onClose = vi.fn();
    render(<PasteModal open onClose={onClose} onSend={vi.fn()} />);
    fireEvent.keyDown(window, { key: 'Escape' });
    expect(onClose).toHaveBeenCalled();
  });

  it('clicking backdrop closes the modal but content click does not', () => {
    const onClose = vi.fn();
    render(<PasteModal open onClose={onClose} onSend={vi.fn()} />);
    // Click on backdrop (the dialog element itself).
    fireEvent.click(screen.getByRole('dialog'));
    expect(onClose).toHaveBeenCalledTimes(1);

    onClose.mockReset();
    fireEvent.click(screen.getByPlaceholderText(/Type or paste/i));
    expect(onClose).not.toHaveBeenCalled();
  });

  it('refuses to send when content exceeds size cap', () => {
    const onSend = vi.fn();
    render(<PasteModal open onClose={vi.fn()} onSend={onSend} />);
    const ta = screen.getByPlaceholderText(/Type or paste/i) as HTMLTextAreaElement;
    // 33KB ASCII exceeds the 32KB cap.
    fireEvent.change(ta, { target: { value: 'A'.repeat(33 * 1024) } });
    const send = screen.getByRole('button', { name: /^Send$/ }) as HTMLButtonElement;
    expect(send.disabled).toBe(true);
    fireEvent.click(send);
    expect(onSend).not.toHaveBeenCalled();
  });

  it('textarea opts out of browser autofill (TM-FRONT.11)', () => {
    // TM-FRONT.11 forward-looking. Paste contents are arbitrary user-typed
    // text — including past prompts, snippets, and occasionally secrets.
    // Browser autofill / OS keyboard suggestion strips happily inject
    // entries from prior sessions across same-origin contexts; the
    // explicit opt-out is the only safeguard. A regression that flips
    // the attribute on (`autocomplete="on"` is the HTML default for
    // textareas) must fail this test.
    render(<PasteModal open onClose={vi.fn()} onSend={vi.fn()} />);
    const ta = screen.getByPlaceholderText(/Type or paste/i) as HTMLTextAreaElement;
    expect(ta.getAttribute('autocomplete')).toBe('off');
    // Sanity: the sibling typing-discipline opt-outs must not regress
    // alongside autoComplete - they were here first and are load-bearing
    // for the iOS pasteboard UX. Bundling them defends the whole cluster.
    expect(ta.getAttribute('autocapitalize')).toBe('off');
    expect(ta.getAttribute('autocorrect')).toBe('off');
    expect(ta.getAttribute('spellcheck')).toBe('false');
  });

  it('does not leak modal content to storage', () => {
    const SECRET = 'my-paste-content-' + 'X'.repeat(20);
    render(<PasteModal open onClose={vi.fn()} onSend={vi.fn()} />);
    const ta = screen.getByPlaceholderText(/Type or paste/i);
    fireEvent.change(ta, { target: { value: SECRET } });
    // Walk both storages and assert SECRET appears nowhere.
    for (let i = 0; i < localStorage.length; i++) {
      const v = localStorage.getItem(localStorage.key(i)!) ?? '';
      expect(v).not.toContain(SECRET);
    }
    for (let i = 0; i < sessionStorage.length; i++) {
      const v = sessionStorage.getItem(sessionStorage.key(i)!) ?? '';
      expect(v).not.toContain(SECRET);
    }
  });
});
