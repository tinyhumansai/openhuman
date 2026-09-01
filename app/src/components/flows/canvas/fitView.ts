/**
 * Shared `fitView` options for the two workflow canvases.
 *
 * React Flow's default is `{ padding: 0.1, maxZoom: 1 }`, so a small graph
 * opens at 100% and three or four cards fill the viewport. `maxZoom` 0.85
 * pulls back far enough to read the shape of a flow on open while the 13px
 * node titles stay legible, and the wider padding keeps the outermost cards
 * off the canvas edge.
 *
 * Its own module rather than an export from `FlowCanvas`: that file already
 * imports `EditableFlowCanvas` (it delegates to it when `editable`), so the
 * editable canvas importing back would close a cycle.
 *
 * Both canvases must fit identically. `EditableFlowCanvas` persists its
 * viewport and restores it on the next mount, so a different initial zoom
 * there would be captured once and then kept forever.
 */
export const FLOW_FIT_VIEW_OPTIONS = { padding: 0.2, maxZoom: 0.85 } as const;
