import { useParams } from 'react-router-dom';

export function SessionPage() {
  const { token } = useParams<{ token: string }>();
  if (!token || token.length !== 43) {
    return <div className="p-4 text-claude-err">Bad token format.</div>;
  }
  return (
    <div className="h-full flex flex-col">
      <header className="px-3 py-2 border-b border-claude-panelBorder text-sm">
        Claude Phone — session pending
      </header>
      <main className="flex-1 p-3">
        Terminal coming in Milestone 6.
        <div className="text-claude-muted text-xs mt-2 break-all">token: {token}</div>
      </main>
    </div>
  );
}
