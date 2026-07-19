/**
 * TypeScript mirror of the `tinyflows` workflow model (`tinyflows::model`,
 * currently pinned at 0.3.0 — see root `Cargo.toml`). This is the frontend's
 * only view onto the wire shape of `Flow.graph` (kept as `unknown` in
 * `services/api/flowsApi.ts` since the list page never needs to interpret
 * it) — the canvas (issue B5b) is the first consumer.
 *
 * Field names/optionality are checked directly against
 * `tinyflows-0.3.0/src/model/mod.rs` and `node_kind.rs` (no `rename_all` on
 * the Rust structs, so field names are snake_case on the wire as-is, same
 * convention as `flowsApi.ts`'s wire types). Two deliberate deviations from a
 * naive "camelCase everything" TS port, both intentional so this file stays a
 * faithful mirror instead of drifting from the Rust source of truth:
 *
 * - `Port` has no `kind: 'input' | 'output'` discriminant — the Rust struct is
 *   just `{ name, label? }`. `Node.ports` is documented on the Rust side as
 *   "Declared output ports (for branching / multi-output nodes)" — i.e. it is
 *   *only* ever a list of a node's extra output ports, never inputs. Nodes
 *   with a single default output (most kinds) leave it empty; a `switch`
 *   node's *actual* case ports are computed at runtime from its `config`
 *   (see `tinyflows`'s `SwitchNode::execute`) and aren't guaranteed to appear
 *   here at all — the graph's `edges` are the only fully authoritative source
 *   for which ports are actually wired. `graphAdapter.ts` accounts for this by
 *   deriving the *effective* input/output handles for the canvas from a union
 *   of `Node.ports` and the edges touching that node, rather than trusting
 *   `Node.ports` alone.
 * - `Position` is `{ x: number; y: number }` (both required numbers), matching
 *   the Rust struct exactly; it's `Node.position` itself that is optional
 *   (`Option<Position>`), not the fields within it.
 */

/** Canvas coordinates for a node. Mirrors `tinyflows::model::Position`. */
export interface Position {
  x: number;
  y: number;
}

/**
 * The 12 node kinds `tinyflows` currently defines (`tinyflows::model::NodeKind`).
 * Wire values are `snake_case` (`#[serde(rename_all = "snake_case")]`).
 */
export type NodeKind =
  | 'trigger'
  | 'agent'
  | 'tool_call'
  | 'http_request'
  | 'code'
  | 'condition'
  | 'switch'
  | 'merge'
  | 'split_out'
  | 'transform'
  | 'output_parser'
  | 'sub_workflow';

/**
 * A named connection point on a node. Mirrors `tinyflows::model::Port`.
 * Despite the name, only ever appears in `WorkflowNode.ports`, which the Rust
 * doc comment describes as "Declared output ports (for branching /
 * multi-output nodes)" — see the module doc above for why this is not a
 * complete picture of a node's input/output handles.
 */
export interface Port {
  name: string;
  label?: string;
}

/**
 * A single unit of work in a workflow. Mirrors `tinyflows::model::Node`
 * (named `WorkflowNode` here to avoid colliding with DOM's global `Node`
 * type and to pair with `WorkflowGraph`/`WorkflowEdge`).
 */
export interface WorkflowNode {
  id: string;
  kind: NodeKind;
  /** Defaults to `1` on the wire (`#[serde(default = "default_type_version")]`). */
  type_version?: number;
  name: string;
  /** Kind-specific configuration as free-form JSON (`#[serde(default)]` → `null`/`{}`). */
  config: Record<string, unknown>;
  /** Declared *output* ports (see module doc). Defaults to `[]` on the wire. */
  ports: Port[];
  /** Canvas position, if the graph was authored/saved with one. */
  position?: Position;
}

/**
 * A directed connection from one node's output port to another's input port.
 * Mirrors `tinyflows::model::Edge`. `from_port`/`to_port` default to
 * `"main"` on the wire when omitted, but the JSON `flows_get` returns has
 * already gone through `tinyflows`'s serializer, so both are always present
 * by the time this client sees them.
 */
export interface WorkflowEdge {
  from_node: string;
  from_port: string;
  to_node: string;
  to_port: string;
}

/**
 * A complete, serializable workflow definition. Mirrors
 * `tinyflows::model::WorkflowGraph`. This is the shape `Flow.graph` (kept
 * `unknown` in `flowsApi.ts`) must be cast to once loaded.
 */
export interface WorkflowGraph {
  /** Overall model-shape version; `tinyflows::model::CURRENT_SCHEMA_VERSION` is `1`. */
  schema_version: number;
  /** Optional stable id of the workflow (`Option<String>` on the Rust side). */
  id?: string | null;
  name: string;
  nodes: WorkflowNode[];
  edges: WorkflowEdge[];
}
