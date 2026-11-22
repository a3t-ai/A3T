// @ts-check
import { defineConfig } from 'astro/config';
import starlight from '@astrojs/starlight';

export default defineConfig({
	site: 'https://docs.a3t.io',
	// Local preview behind a path proxy (code-server /absproxy/<port>); unset for the real build.
	...(process.env.DOCS_BASE ? { base: process.env.DOCS_BASE } : {}),
	integrations: [
		starlight({
			title: 'A3T Docs',
			social: [
				{ icon: 'github', label: 'GitHub', href: 'https://github.com/openagen/flowr' },
			],
			customCss: ['./src/styles/custom.css'],
			components: { Footer: './src/components/Footer.astro' },
			editLink: { baseUrl: 'https://github.com/openagen/flowr/edit/main/a3t/wwwdocs/' },
			sidebar: [
				{
					label: 'Getting Started',
					items: [
						{ label: 'Why A3T', slug: 'getting-started/why-a3t' },
						{ label: 'How it works', slug: 'getting-started/architecture' },
						{ label: 'Quick start', slug: 'getting-started/quickstart' },
					],
				},
				{
					label: 'Concepts',
					items: [
						{ label: 'YAML', slug: 'concepts/yaml' },
						{ label: 'CNCF Serverless Workflow', slug: 'concepts/cncf-swf' },
						{ label: 'Workflows', slug: 'concepts/workflows' },
						{ label: 'Task Types', slug: 'concepts/task-types' },
						{ label: 'Resources', slug: 'concepts/resources' },
						{ label: 'Expressions', slug: 'concepts/expressions' },
						{ label: 'Agents', slug: 'concepts/agents' },
						{ label: 'Memory', slug: 'concepts/memory' },
						{ label: 'AQL', slug: 'concepts/aql' },
						{ label: 'Triggers', slug: 'concepts/triggers' },
					],
				},
				{
					label: 'Governance',
					items: [
						{ label: 'Overview', slug: 'governance/overview' },
						{ label: 'Identity and roles', slug: 'governance/identity-and-roles' },
						{ label: 'Policy files', slug: 'governance/policy-files' },
						{ label: 'Permission catalogue', slug: 'governance/permissions' },
						{ label: 'Writing a rule', slug: 'governance/writing-a-rule' },
						{ label: 'Agents and approvals', slug: 'governance/agents-and-approvals' },
					],
				},
				{
					label: 'A3T Console',
					items: [
						{ label: 'Get access and sign in', slug: 'console/access' },
						{
							label: 'Home',
							items: [
								{ label: 'AI Chat', slug: 'console/chat' },
								{ label: 'Projects', slug: 'console/projects' },
							],
						},
						{
							label: 'Agents',
							items: [
								{ label: 'Agents', slug: 'console/agents' },
								{ label: 'Templates', slug: 'console/templates' },
								{ label: 'Triggers', slug: 'console/triggers' },
								{ label: 'Canvas', slug: 'console/canvas' },
								{ label: 'Resources', slug: 'console/resources' },
							],
						},
						{
							label: 'Knowledge & data',
							items: [
								{ label: 'Files', slug: 'console/files' },
								{ label: 'Memory', slug: 'console/memory' },
								{ label: 'Knowledge', slug: 'console/knowledge' },
							],
						},
						{
							label: 'Security & audit',
							items: [
								{ label: 'Security', slug: 'console/security' },
								{ label: 'Decision log', slug: 'console/decision-log' },
							],
						},
						{
							label: 'Runtimes & runs',
							items: [
								{ label: 'Runtimes', slug: 'console/runtimes' },
								{ label: 'Executions', slug: 'console/executions' },
							],
						},
						{
							label: 'Settings',
							items: [
								{ label: 'Studio', slug: 'console/studio' },
								{ label: 'Settings', slug: 'console/settings' },
							],
						},
					],
				},
				{
					label: 'Task Reference',
					autogenerate: { directory: 'tasks' },
				},
				{
					label: 'API Reference',
					autogenerate: { directory: 'api' },
				},
				{
					label: 'Guides',
					autogenerate: { directory: 'guides' },
				},
			],
		}),
	],
});
