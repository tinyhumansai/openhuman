/**
 * `/dev/assistant-ui` — the upstream assistant-ui `base` demo, vendored.
 *
 * Source: https://www.assistant-ui.com/demos/base
 * (`apps/docs/components/pages/examples/base.tsx` in assistant-ui/assistant-ui)
 *
 * Vendored verbatim apart from the host-specific bits that cannot survive the
 * move: Next's `Image`/`next/public` asset import, the docs site's live model
 * catalogue, and the shadcn `@/components/ui` + `@/lib/utils` aliases, which
 * this repo scopes under `@/components/assistant-ui`. Keeping the rest byte-for-
 * byte is the point — it is a reference for what the primitives can render, so
 * a local rewrite would make it stop being one.
 *
 * The runtime under it is mock-only (see `MockRuntimeProvider`).
 */
import { Base } from './BaseDemo';
import { MockRuntimeProvider } from './MockRuntimeProvider';

export function AssistantUiDemoPage() {
  return (
    <div className="h-dvh w-full overflow-hidden">
      <MockRuntimeProvider>
        <Base />
      </MockRuntimeProvider>
    </div>
  );
}

export default AssistantUiDemoPage;
