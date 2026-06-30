export function chatThreadPath(threadId: string): string {
  return `/chat/${encodeURIComponent(threadId)}`;
}
