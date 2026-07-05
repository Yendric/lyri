<script>
  const track = {
    title: 'Holiday',
    artist: 'Green Day',
    duration: 232,
    source: 'Lyrics from LRCLIB',
    lines: [
      { id: 'cha', timestamp: 14, text: 'Cha!' },
      { id: 'falling-rain', timestamp: 23, text: 'Hear the sound of the falling rain' },
      { id: 'armageddon', timestamp: 29, text: 'Coming down like an Armageddon flame (hey!)' },
      { id: 'shame', timestamp: 34, text: 'The shame, the ones who died without a name' },
      { id: 'dogs', timestamp: 40, text: 'Hear the dogs howling out of key' },
      { id: 'hymn', timestamp: 47, text: 'To a hymn called "Faith and Misery" (hey!)' },
      { id: 'company', timestamp: 54, text: 'And bleed, the company lost the war today, and' },
      { id: 'dream', timestamp: 61, text: 'Oh I beg to dream and differ from the hollow lies' },
      {
        id: 'dawning',
        timestamp: 68,
        text: 'This is the dawning of the rest of our lives (whoa, oh, whoa, oh)'
      },
      { id: 'holiday', timestamp: 76, text: 'On holiday!' }
    ]
  };

  let theme = $state('dark');
  let position = $state(35);
  let innerWidth = $state(1280);
  let lyricViewport;

  // -1 until the first line's timestamp: no line is highlighted yet.
  let activeIndex = $derived(track.lines.findLastIndex((line) => line.timestamp <= position));

  let themeIcon = $derived(theme === 'dark' ? '\u2600\uFE0F' : '\uD83C\uDF19');
  let contentPadding = $derived(responsiveContentPadding(innerWidth));
  let viewportWidth = $derived(Math.max(innerWidth - contentPadding * 2, 220));
  let progressLabel = $derived(`${formatTime(position)} / ${formatTime(track.duration)}`);

  $effect(() => {
    const timer = setInterval(() => {
      position = position >= track.duration ? 0 : position + 0.25;
    }, 250);

    return () => clearInterval(timer);
  });

  // $effect runs after the DOM has been updated, so the active line's
  // element already reflects the latest activeIndex.
  $effect(() => {
    const activeLine = lyricViewport?.querySelector(`[data-lyric-index="${activeIndex}"]`);
    activeLine?.scrollIntoView({
      behavior: 'smooth',
      block: 'center',
      inline: 'nearest'
    });
  });

  function formatTime(seconds) {
    const safeSeconds = Math.max(0, Math.floor(seconds));
    const minutes = Math.floor(safeSeconds / 60);
    const remainingSeconds = safeSeconds % 60;

    return `${minutes}:${remainingSeconds.toString().padStart(2, '0')}`;
  }

  function lyricLineStyle(isActive) {
    const fontSize = isActive
      ? responsiveActiveFont(viewportWidth)
      : responsiveInactiveFont(viewportWidth);

    return [
      `--line-width: ${responsiveLyricTextWidth(viewportWidth, fontSize)}px`,
      `--line-font-size: ${fontSize}px`,
      `--line-min-height: ${responsiveMinLineHeight(viewportWidth)}px`
    ].join('; ');
  }

  // Breakpoints must match the .lyrics-screen media queries in app.css.
  function responsiveContentPadding(width) {
    if (width <= 720) {
      return 20;
    }

    return width >= 1400 ? 48 : 36;
  }

  function responsiveLyricTextWidth(width, fontSize) {
    return (responsiveActiveLyricTextWidth(width) * fontSize) / responsiveActiveFont(width);
  }

  function responsiveActiveLyricTextWidth(width) {
    return Math.max(width * 0.94, 360);
  }

  function responsiveActiveFont(width) {
    return width >= 1400 ? 38 : 28;
  }

  function responsiveInactiveFont(width) {
    return width >= 1400 ? 26 : 20;
  }

  function responsiveMinLineHeight(width) {
    return width >= 1400 ? 72 : 56;
  }

  function toggleTheme() {
    theme = theme === 'dark' ? 'light' : 'dark';
  }
</script>

<svelte:window bind:innerWidth />

<svelte:head>
  <meta name="theme-color" content={theme === 'dark' ? '#1b1c20' : '#eef2f8'} />
</svelte:head>

<div class={['lyri-shell', theme]} aria-label="Lyri live lyrics mockup">
  <header class="titlebar">
    <div class="toolbar" aria-label="Theme controls">
      <button
        class="icon-button"
        type="button"
        aria-label={theme === 'dark' ? 'Light mode' : 'Dark mode'}
        title={theme === 'dark' ? 'Light mode' : 'Dark mode'}
        onclick={toggleTheme}
      >
        {themeIcon}
      </button>
    </div>
  </header>

  <main class="lyrics-screen">
    <section class="track-header" aria-label="Now playing">
      <h1>{track.title}</h1>
      <p>{track.artist}</p>
      <time>{progressLabel}</time>
    </section>

    <section class="lyrics-viewport" bind:this={lyricViewport} aria-label={track.source}>
      <div class="lyrics-stack">
        {#each track.lines as line, index (line.id)}
          {@const distance = activeIndex < 0 ? index + 1 : Math.abs(index - activeIndex)}
          <p
            class={[
              'lyric-line',
              distance === 0 && 'active',
              distance === 1 && 'near',
              distance === 2 && 'far',
              distance > 2 && 'distant'
            ]}
            data-lyric-index={index}
            style={lyricLineStyle(distance === 0)}
          >
            {line.text}
          </p>
        {/each}
      </div>
    </section>
  </main>
</div>
