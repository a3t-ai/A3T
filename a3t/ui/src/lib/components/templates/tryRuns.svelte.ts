// Try it runs, per template, for this visit. The guide is rebuilt on every tab
// switch; keeping the runs here means the side-by-side comparison and
// Production's "Worked when tried" survive moving between tabs.

export type TryRun = {
  ok: boolean;
  text: string;
  ms: number;
  toolCalls?: number;
  stopped?: string;
  input: string;
  agent: string;
  at: number;
};

let runs = $state<Record<string, TryRun[]>>({});

export function runsOf(templateId: string | undefined): TryRun[] {
  return (templateId && runs[templateId]) || [];
}

/// Newest first; the last six are kept.
export function addRun(templateId: string, r: TryRun) {
  runs = { ...runs, [templateId]: [r, ...(runs[templateId] ?? [])].slice(0, 6) };
}
