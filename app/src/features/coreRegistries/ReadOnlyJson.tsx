interface ReadOnlyJsonProps {
  value: unknown;
}

function stringifyJson(value: unknown): string {
  try {
    return JSON.stringify(value, null, 2);
  } catch {
    return String(value);
  }
}

export default function ReadOnlyJson({ value }: ReadOnlyJsonProps) {
  return (
    <pre className="overflow-x-auto rounded-2xl border border-stone-200 bg-stone-950 px-4 py-3 text-xs leading-6 text-stone-100">
      <code>{stringifyJson(value)}</code>
    </pre>
  );
}
