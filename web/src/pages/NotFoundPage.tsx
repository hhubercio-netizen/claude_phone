export function NotFoundPage() {
  return (
    <div className="p-4">
      <h1 className="text-lg">Not found</h1>
      <p className="text-claude-muted">
        This page does not exist. To start a session, type <code>/phone</code> in your
        Claude Code terminal and scan the QR.
      </p>
    </div>
  );
}
