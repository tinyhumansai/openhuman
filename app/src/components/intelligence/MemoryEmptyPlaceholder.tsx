/**
 * Right-pane placeholder shown to brand-new users (zero chunks).
 *
 * Centered, generous whitespace, no call-to-action buttons — the only path
 * forward is connecting an integration in Settings, so we point there in
 * prose without an explicit link to keep the surface meditative.
 */
export function MemoryEmptyPlaceholder() {
  return (
    <div className="mw-detail-empty" data-testid="memory-empty-placeholder">
      <h2 className="mw-empty-title">Nothing yet.</h2>
      <p className="mw-empty-body">
        Connect an integration in Settings to start
        <br />
        building your memory tree.
      </p>
    </div>
  );
}
