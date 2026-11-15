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
    headline: 'Organise work into projects.', highlight: 'projects', docsPath: '/console/projects/',
    subtitle: 'A project holds a team\'s agents, flows and connections, and decides who can see and change them.',
    description: 'Projects you create sit under your name; a company\'s sit under its name. Pick a project in the top bar and every app shows only that project\'s work.',
  },
  agents: {
    headline: 'Your AI agents, ready for your apps.', highlight: 'agents', docsPath: '/console/agents/',
    subtitle: 'An agent is an AI assistant that your apps talk to. Each one has its own model, instructions, tools and memory.',
    description: 'Chat with an agent to try it, see how much it is used, change how it behaves, or copy it to start your own.',
  },
  templates: {
    headline: 'Every agent starts from a template.', highlight: 'template', docsPath: '/console/templates/',
    subtitle: 'A template is a ready-made recipe for an agent: how it handles each message, and which tools it can reach for.',
    description: 'Read how a template works, see the agents made from it, or make a new agent from it.',
  },
  triggers: {
    headline: 'Run agents and flows automatically.', highlight: 'automatically', docsPath: '/console/triggers/',
    subtitle: 'A trigger starts something on its own: on a schedule, when another system calls a web address, or when an event arrives.',
    description: 'See what sets each trigger off and what it starts, try it with a test event, or pause it.',
  },
  canvas: {
    headline: 'Choose where and how a flow runs.', highlight: 'runs', docsPath: '/console/canvas/',
    subtitle: 'A flow is a series of steps that an agent or system carries out. Its canvas holds the run settings: which server, how many copies, and what it needs.',
    description: 'See where each flow will run, deploy it, and check that it is running.',
  },
  resources: {
    headline: 'Connect the services your agents use.', highlight: 'services', docsPath: '/console/resources/',
    subtitle: 'Databases, AI models, tool servers, file storage, and keys and passwords — each set up once.',
    description: 'Agents and flows use a resource by its name, so its details live in one place. See its settings, test the connection, and see what uses it.',
  },
  datarooms: {
    headline: 'Files your agents can read and write.', highlight: 'Files', docsPath: '/console/files/',
    subtitle: 'Cloud storage and file servers connected to A3T: Amazon S3, MinIO, Google Cloud, Azure and FTP.',
    description: 'Browse the files in each one, and see which agents and flows use it.',
  },
  memory: {
    headline: 'What your agents remember.', highlight: 'remember', docsPath: '/console/memory/',
    subtitle: 'Each agent keeps its current conversation, its past conversations, facts it has learned and steps it knows — plus any shared knowledge it is given.',
    description: 'Look through and search an agent\'s memory. To change what it remembers, go to the agent itself.',
  },
  knowledge: {
    headline: 'Explore what your agents know.', highlight: 'know', docsPath: '/console/knowledge/',
    subtitle: 'A knowledge graph is a map of people, things and ideas and how they are linked, built from facts your agents have learned.',
    description: 'Explore a graph, ask it questions, find links that are likely but missing, or rebuild it with the latest facts.',
  },
  security: {
    headline: 'Decide who can do what.', highlight: 'who', docsPath: '/console/security/',
    subtitle: 'Access rules for each project, on top of the platform\'s defaults, for the people in your company\'s sign-in. For security admins.',
    description: 'Read a project\'s rules, change them — they apply straight away — and test a decision before it matters.',
  },
  logs: {
    headline: 'See every access decision.', highlight: 'decision', docsPath: '/console/decision-log/',
    subtitle: 'Each time a person or an agent tried to do something: who, what, whether it was allowed, and which rule decided. For security admins.',
    description: 'It shows the project picked in the top bar; pick All projects to see everything.',
  },
  runtime: {
    headline: 'See what is running, and where.', highlight: 'running', docsPath: '/console/runtimes/',
    subtitle: 'The servers A3T runs on, and the agents and flows working on each one.',
    description: 'Check each server\'s health and load, and pause, restart or stop what runs there.',
  },
  executions: {
    headline: 'Every run, step by step.', highlight: 'run', docsPath: '/console/executions/',
    subtitle: 'Each time an agent or a flow ran: when, what started it, and how long each step took.',
    description: 'See every step, what went in and what came out, and why a run failed if it did. Run it again from here.',
  },
  settings: {
    headline: 'Set up your platform.', highlight: 'platform', docsPath: '/console/settings/',
    subtitle: 'Sign-in, security, servers and billing, in one place.',
    description: 'In Community Edition anyone who signs in can change things. Enterprise adds roles, access rules, company sign-in and an audit trail; those settings are marked EE.',
  },
  'early-access': {
    headline: 'People asking to join.', highlight: 'join', docsPath: '',
    subtitle: 'Requests to try A3T early, sent from the sign-in page.',
    description: 'See who asked and why, and get in touch with the right people.',
  },
};

/// PageBanner's props for an app: `<PageBanner {...bannerFor('canvas')} />`.
export function bannerFor(app: string, fallbackTitle = '') {
  const b = BANNERS[app];
  return b
    ? { headline: b.headline, highlightWord: b.highlight, subtitle: b.subtitle, description: b.description, docsPath: b.docsPath, color: appColor(app) }
    : { headline: fallbackTitle, highlightWord: '', subtitle: '', description: '', docsPath: '', color: appColor(app) };
}
