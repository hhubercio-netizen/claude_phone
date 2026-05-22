interface Props {
  error: unknown;
}

export function ErrorPage({ error }: Props) {
  const msg = error instanceof Error ? error.message : String(error);
  return (
    <div className="p-4">
      <h1 className="text-lg text-claude-err">Something went wrong</h1>
      <pre className="mt-2 text-xs whitespace-pre-wrap">{msg}</pre>
      <button
        className="mt-3 px-3 py-1 border border-claude-panelBorder rounded"
        onClick={() => window.location.reload()}
      >
        Reload
      </button>
    </div>
  );
}
