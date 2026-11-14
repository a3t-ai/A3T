// Each app's own colour and its banner, in one place. The colour marks the app
// everywhere it shows: the banner's tint and highlighted word, the icons in its
// Items list, its cards on an app home. The banner says what the app is for, in
// the same shape for every app:
//   headline     one sentence; `highlight` is the word set in italic, in colour
//   subtitle     what the thing is
//   description  what you can do with it here
// The Details pane adds "Pick from … in Items" under it, so the text doesn't.

export interface AppBanner {
  headline: string;
  highlight: string;
  subtitle: string;
  description: string;
  /// A page on docs.a3t.io that covers it; '' when there is none yet (no link shown).
  docsPath: string;
}

/// One colour per app — mid tones, so they read on the light and the dark theme.
export const APP_COLOR: Record<string, string> = {
  chat: '#3a5a85',
  projects: '#6d28d9',
  agents: '#1a7158',
  templates: '#4338ca',
  triggers: '#0f766e',
  canvas: '#c2410c',
  resources: '#2563eb',
  datarooms: '#a16207',
  memory: '#be185d',
  knowledge: '#a21caf',
  security: '#b42318',
  logs: '#475569',
  runtime: '#0369a1',
  executions: '#4d7c0f',
  'flow-editor': '#5530a8',
  settings: '#57534e',
  'early-access': '#9333ea',
};

export const appColor = (app: string) => APP_COLOR[app] ?? '#57534e';

export const BANNERS: Record<string, AppBanner> = {
  projects: {
    headline: 'Organise work into projects.', highlight: 'projects', docsPath: '',
    subtitle: 'Each project is a namespace: its agents, flows and resources, the people in it, and the rules they run under.',
    description: 'A project you create lives under your handle; an organisation\'s lives under its name. Make it private to keep it to yourself or its members. Pick one in the top bar and every app shows only its work.',
  },
  agents: {
    headline: 'Agents, over an API.', highlight: 'Agents', docsPath: '/concepts/agents/',
    subtitle: 'Each one made from a template, or cloned, forked or branched from another agent.',
    description: 'An agent has a data plane — chat, streamed — for the apps that call it, and a control plane for its config, runtime and memory. See its figures, history, versions and branches.',
  },
  templates: {
    headline: 'Every agent starts from a template.', highlight: 'template', docsPath: '',
    subtitle: 'A template is code: a flow that runs each turn on the core agent, with its hooks and tools.',
    description: 'The template says how a turn runs; each agent made from it fills in its own model, instructions, tools and memory. Read its code, see the agents made from it, or make one.',
  },
  triggers: {
    headline: 'Start flows and agents from events.', highlight: 'events', docsPath: '/concepts/triggers/',
    subtitle: 'A webhook call, a schedule, a message on an ADB topic or a platform event.',
    description: 'A trigger is YAML: what fires it, and where each firing goes, with the input you map. Pause or rewire it without touching the flow.',
  },
  canvas: {
    headline: 'Deploy a flow with a canvas.', highlight: 'canvas', docsPath: '',
    subtitle: 'A canvas is a flow\'s deployment file: where it loads, how it runs, and what it depends on.',
    description: 'It is YAML next to its flow. See which runtime takes it, deploy it, and watch it run.',
  },
  resources: {
    headline: 'Connect your resources.', highlight: 'resources', docsPath: '/concepts/resources/',
    subtitle: 'Databases, LLMs, MCP servers, storage and secrets — everything agents and flows use.',
    description: 'Register each one once, then refer to it by name from any flow or agent. See its configuration, test it, and see what uses it.',
  },
  datarooms: {
    headline: 'Files, where agents read and write them.', highlight: 'Files', docsPath: '',
    subtitle: 'S3 and MinIO buckets, Google Cloud Storage, Azure containers and FTP servers — each one a storage resource.',
    description: 'See where each one points, which agents and flows use it, and browse its files through its own driver.',
  },
  memory: {
    headline: 'What each agent remembers.', highlight: 'remembers', docsPath: '/concepts/memory/',
    subtitle: 'Working, episodic, semantic and procedural memory, kept in ADB — and the shared spaces each agent reads.',
    description: 'See each kind of an agent\'s memory, filter it, or read it with AQL. Changing what an agent remembers is done on the agent, so its checks apply.',
  },
  knowledge: {
    headline: 'Knowledge graphs, built from what agents know.', highlight: 'Knowledge', docsPath: '',
    subtitle: 'Each one a store of entities and relations with their RotatE embeddings, built from a semantic space in ADB.',
    description: 'See what a graph holds, explore it, query its triples and predict missing links, or build it again.',
  },
  security: {
    headline: 'Who may do what, project by project.', highlight: 'project', docsPath: '',
    subtitle: 'Cedar policies per project over the compiled baseline, with identity from your sign-in provider. SuperAdmin and SecurityAdmin only.',
    description: 'See what a policy permits and forbids, edit its Cedar — applied live — and dry-run a decision before it matters.',
  },
  logs: {
    headline: 'Every decision, allowed or refused.', highlight: 'decision', docsPath: '',
    subtitle: 'Each call at a choke point: who, on whose behalf, what, which policy, and why. SuperAdmin and SecurityAdmin only.',
    description: 'It follows the project in the top bar; choose All projects to see every one.',
  },
  runtime: {
    headline: 'Operate what is running, where it runs.', highlight: 'running', docsPath: '',
    subtitle: 'Hosts, the runtimes on them, and the flows and agents deployed on each.',
    description: 'See health and work pools; pause, resume, restart, undeploy or terminate. How something deploys is set in its canvas.',
  },
  executions: {
    headline: 'Every run, step by step.', highlight: 'run', docsPath: '/concepts/workflows/',
    subtitle: 'What ran, when, what started it, and how long each step took.',
    description: 'See a run\'s steps, timeline, journal, input and output — and why it failed, if it did. Re-run it, or re-run it in debug from a resume point.',
  },
  settings: {
    headline: 'Configure your platform.', highlight: 'platform', docsPath: '',
    subtitle: 'Identity, security, infrastructure and billing, in one place.',
    description: 'Community Edition runs open: anyone who signs in can edit. Enterprise adds roles, Cedar policy, SSO and audit; its settings are marked EE.',
  },
  'early-access': {
    headline: 'Who wants in.', highlight: 'wants', docsPath: '',
    subtitle: 'Early-access requests sent from the sign-in page.',
    description: 'Every request is kept in the early_access_requests table. Reach out from here to bring the right people in.',
  },
};

/// PageBanner's props for an app: `<PageBanner {...bannerFor('canvas')} />`.
export function bannerFor(app: string, fallbackTitle = '') {
  const b = BANNERS[app];
  return b
    ? { headline: b.headline, highlightWord: b.highlight, subtitle: b.subtitle, description: b.description, docsPath: b.docsPath, color: appColor(app) }
    : { headline: fallbackTitle, highlightWord: '', subtitle: '', description: '', docsPath: '', color: appColor(app) };
}
