import type { ArtifactSnapshot } from '../../store/chatRuntimeSlice';

/**
 * Extension hint for the Tauri download / Save-As commands.
 *
 * Prefers an explicit extension already present on the artifact title
 * (defensive — `create_artifact` sanitises the title + extension
 * separately, but a malformed title shouldn't crash the card), otherwise
 * falls back to a per-kind default.
 *
 * Shared by {@link ArtifactCard} and {@link ChatFilesPanel}, which both
 * derive the download filename extension from `(kind, title)` and must
 * stay in lockstep — hence a single source of truth rather than two
 * hand-kept copies.
 *
 * `document` → `docx`: the document artifact producer (`generate_document`,
 * GH #4847) emits a real Word `.docx`. The prior `pdf` default predated any
 * document producer and would have handed the byte-agnostic export a wrong
 * extension.
 */
export function extensionFor(kind: ArtifactSnapshot['kind'], title: string): string {
  const dot = title.lastIndexOf('.');
  if (dot > 0 && dot < title.length - 1) {
    return title.slice(dot + 1).toLowerCase();
  }
  switch (kind) {
    case 'presentation':
      return 'pptx';
    case 'document':
      return 'docx';
    case 'image':
      return 'png';
    default:
      return 'bin';
  }
}
