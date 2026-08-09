<script>
  const RECONNECT_DELAY_MS = 3000;

  let theme = $state('dark');
  let connected = $state(false);
  let view = $state(null); // last ViewState from the host
  let receivedAt = $state(0); // performance.now() at receipt
  let now = $state(0); // ticked while playing to extrapolate the position
  let innerWidth = $state(1280);
  // $state so effects reading it re-run when the conditionally rendered
  // lyrics viewport mounts and binds.
  let lyricViewport = $state();
  let socket;

  // Follow the hosting Lyri instance over its WebSocket, reconnecting for
  // as long as the page is open.
  $effect(() => {
    let disposed = false;
    let retry;

    const connect = () => {
      const protocol = location.protocol === 'https:' ? 'wss' : 'ws';
      socket = new WebSocket(`${protocol}://${location.host}/ws`);
      socket.onopen = () => {
        connected = true;
      };
      socket.onmessage = (event) => {
        const message = JSON.parse(event.data);
        if (message.type === 'state') {
          view = message;
          receivedAt = performance.now();
          now = receivedAt;
        }
      };
      socket.onclose = () => {
        connected = false;
        view = null;
        if (!disposed) {
          retry = setTimeout(connect, RECONNECT_DELAY_MS);
        }
      };
    };

    connect();
    return () => {
      disposed = true;
      clearTimeout(retry);
      socket?.close();
    };
  });

  let playback = $derived(view?.playback ?? null);
  let lyrics = $derived(view?.lyrics ?? null);

  $effect(() => {
    if (!playback?.isPlaying) {
      return;
    }

    const timer = setInterval(() => {
      now = performance.now();
    }, 250);

    return () => clearInterval(timer);
  });

  let positionMs = $derived.by(() => {
    if (!playback) {
      return 0;
    }

    const elapsed = playback.isPlaying ? Math.max(0, now - receivedAt) : 0;
    const position = playback.positionMs + elapsed;
    return playback.durationMs == null ? position : Math.min(position, playback.durationMs);
  });

  let lines = $derived(lyrics?.kind === 'synced' ? lyrics.lines : []);
  // -1 until the first line's timestamp: no line is highlighted yet.
  let activeIndex = $derived(lines.findLastIndex((line) => line.timestampMs <= positionMs));

  let status = $derived.by(() => {
    if (!connected) return 'Connecting to Lyri...';
    if (!playback) return 'Listening for media...';
    switch (lyrics?.kind) {
      case 'loading':
        return 'Finding lyrics...';
      case 'missing':
        return 'No lyrics found.';
      case 'error':
        return 'Lyrics lookup failed.';
      case undefined:
        return 'Listening for media...';
      default:
        return `Lyrics from ${lyrics.source}`;
    }
  });

  let emptyMessage = $derived.by(() => {
    if (!connected) return 'Connecting to Lyri...';
    if (!playback) return 'Play a song and Lyri will follow along.';
    switch (lyrics?.kind) {
      case 'missing':
        return 'No lyrics found for this track.';
      case 'error':
        return lyrics.message || 'Could not load lyrics';
      case 'instrumental':
        return 'Instrumental track';
      default:
        return 'Finding lyrics...';
    }
  });

  let title = $derived(playback?.title.trim() ? playback.title : 'Lyri');
  let subtitle = $derived.by(() => {
    if (!playback) return status;
    const artist = playback.artist.trim();
    const album = playback.album.trim();
    if (artist && album) return `${artist} - ${album}`;
    return artist || album || 'Unknown artist';
  });
  let progressLabel = $derived.by(() => {
    if (!playback) return '';
    const position = formatTime(positionMs);
    const suffix = playback.isPlaying ? '' : ' paused';
    return playback.durationMs == null
      ? `${position}${suffix}`
      : `${position} / ${formatTime(playback.durationMs)}${suffix}`;
  });

  let themeIcon = $derived(theme === 'dark' ? '☀️' : '🌙');
  let contentPadding = $derived(responsiveContentPadding(innerWidth));
  let viewportWidth = $derived(Math.max(innerWidth - contentPadding * 2, 220));

  // Centers the active line. Read both reactive values before any early
  // return, or the effect loses its dependencies and never re-runs.
  $effect(() => {
    const index = activeIndex;
    const viewport = lyricViewport;
    const activeLine = viewport?.querySelector(`[data-lyric-index="${index}"]`);
    if (!viewport || !activeLine) {
      return;
    }

    const centered =
      activeLine.offsetTop - (viewport.clientHeight - activeLine.offsetHeight) / 2;
    const top = Math.max(0, centered);
    try {
      viewport.scrollTo({ top });
    } catch {
      // Engines without ScrollToOptions support get an instant jump.
      viewport.scrollTop = top;
    }
  });

  function seekTo(line) {
    if (!playback?.canSeek || socket?.readyState !== WebSocket.OPEN) {
      return;
    }

    socket.send(JSON.stringify({ type: 'seek', positionMs: line.timestampMs }));
  }

  function formatTime(milliseconds) {
    const totalSeconds = Math.max(0, Math.floor(milliseconds / 1000));
    const minutes = Math.floor(totalSeconds / 60);
    const seconds = totalSeconds % 60;

    return `${minutes}:${seconds.toString().padStart(2, '0')}`;
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

<div class={['lyri-shell', theme]} aria-label="Lyri live lyrics">
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
      <h1>{title}</h1>
      <p>{subtitle}</p>
      <time>{progressLabel}</time>
    </section>

    {#if lyrics?.kind === 'synced'}
      <section class="lyrics-viewport" bind:this={lyricViewport} aria-label={status}>
        <div class="lyrics-stack">
          {#each lines as line, index}
            {@const distance = activeIndex < 0 ? index + 1 : Math.abs(index - activeIndex)}
            <p
              class={[
                'lyric-line',
                playback?.canSeek && 'seekable',
                distance === 0 && 'active',
                distance === 1 && 'near',
                distance === 2 && 'far',
                distance > 2 && 'distant'
              ]}
              data-lyric-index={index}
              style={lyricLineStyle(distance === 0)}
              title={playback?.canSeek ? `Jump to ${formatTime(line.timestampMs)}` : undefined}
              onclick={() => seekTo(line)}
            >
              {line.text}
            </p>
          {/each}
        </div>
      </section>
    {:else if lyrics?.kind === 'plain'}
      <section class="lyrics-viewport" aria-label={status}>
        <p class="plain-lyrics">{lyrics.text}</p>
      </section>
    {:else}
      <section class="empty-state">
        <p>{emptyMessage}</p>
      </section>
    {/if}
  </main>
</div>
