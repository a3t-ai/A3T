<script lang="ts">
  /// The intro banner at the top of a section page: what this part of the platform
  /// is for, in the product's own voice.
  ///
  /// `highlightWord` is rendered in italic within the headline, so the headline is
  /// written as one sentence and the emphasis is data rather than markup. Matching
  /// is case-insensitive, and a word that does not occur simply leaves the headline
  /// unstyled instead of breaking it.
  let {
    headline = '',
    highlightWord = '',
    subtitle = '',
    description = '',
    accent = 'spec',
    color = '',
    docsPath = '',
  }: {
    headline?: string;
    highlightWord?: string;
    subtitle?: string;
    description?: string;
    /// One of the platform's four accent families.
    accent?: 'spec' | 'runtime' | 'control' | 'ephemeral';
    /// The app's own colour (shell/appTheme.ts); it wins over `accent`.
    color?: string;
    /// A page on docs.a3t.io; without one there is no "Learn more".
    docsPath?: string;
  } = $props();

  const parts = $derived.by(() => {
    if (!highlightWord) return { before: headline, highlight: '', after: '' };
    const i = headline.toLowerCase().indexOf(highlightWord.toLowerCase());
    if (i === -1) return { before: headline, highlight: '', after: '' };
    return {
      before: headline.slice(0, i),
      highlight: headline.slice(i, i + highlightWord.length),
      after: headline.slice(i + highlightWord.length),
    };
  });

  const ACCENTS: Record<string, { color: string; bg: string }> = {
    spec:      { color: 'var(--spec, #5530a8)',      bg: 'var(--spec-bg, #ede7ff)' },
    runtime:   { color: 'var(--runtime, #1a7158)',   bg: 'var(--runtime-bg, #e3f4ef)' },
    control:   { color: 'var(--control, #c97c2e)',   bg: 'var(--control-bg, #fbf0e0)' },
    ephemeral: { color: 'var(--ephemeral, #3a7fff)', bg: 'var(--ephemeral-bg, #eaf1ff)' },
  };
  /// The tint, border and highlighted word, all from one colour; mixed with the
  /// theme's own surface and text so it reads on light and dark alike.
  const tone = $derived(color
    ? { color: `color-mix(in srgb, ${color} 78%, var(--text-1))`, bg: `color-mix(in srgb, ${color} 11%, var(--bg-surface))`, line: `color-mix(in srgb, ${color} 24%, var(--bg-surface))` }
    : { ...(ACCENTS[accent] ?? ACCENTS.spec), line: (ACCENTS[accent] ?? ACCENTS.spec).bg });

  const docsUrl = $derived(docsPath ? `https://docs.a3t.io${docsPath.startsWith('/') ? '' : '/'}${docsPath}` : '');
</script>

  <!-- Banners sit in panes, which have no close; only pop-ins do. -->
  <div
    class="page-banner"
    style="background: linear-gradient(135deg, {tone.bg} 0%, var(--bg-subtle) 100%); border-color: {tone.line};"
  >

    <h2 class="banner-headline">
      {parts.before}<em class="banner-highlight" style="color: {tone.color};">{parts.highlight}</em>{parts.after}
    </h2>

    {#if subtitle}<p class="banner-subtitle">{subtitle}</p>{/if}
    {#if description}<p class="banner-desc">{description}</p>{/if}

    {#if docsUrl}
    <a class="banner-docs" href={docsUrl} target="_blank" rel="noopener noreferrer" style="color: {tone.color};">
      Learn more
      <svg width="12" height="12" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round">
        <path d="M6 3h7v7M13 3L6 10" />
      </svg>
    </a>
    {/if}
  </div>

<style>
  .page-banner {
    position: relative;
    border: 1px solid;
    border-radius: var(--r);
    padding: 28px 32px 24px;
    margin-bottom: 24px;
  }
  .banner-headline {
    font-family: var(--font-serif);
    font-size: 26px; font-weight: 500; line-height: 1.35;
    letter-spacing: -0.02em;
    color: var(--text-1);
    margin-bottom: 12px;
  }
  .banner-highlight { font-style: italic; }
  .banner-subtitle {
    font-size: 15px; font-weight: 500; line-height: 1.5;
    color: var(--text-2); margin-bottom: 8px;
  }
  .banner-desc {
    font-size: 13px; line-height: 1.6;
    color: var(--text-3); max-width: 640px;
  }
  .banner-docs {
    display: inline-flex; align-items: center; gap: 4px;
    margin-top: 12px;
    font-size: 12px; font-weight: 500;
    text-decoration: none;
    transition: opacity 0.12s;
  }
  .banner-docs:hover { opacity: 0.8; }
</style>
