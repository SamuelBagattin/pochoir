<script lang="ts">
  import { t } from '../i18n';
  import type { PreviewBitmap } from '../engine/module';

  interface Props {
    bitmap: PreviewBitmap | null;
    busy: boolean;
    statusLabel: string;
    /** Announced politely once a render settles. Empty while one is running. */
    announcement: string;
    fileName: string | null;
  }

  let { bitmap, busy, statusLabel, announcement, fileName }: Props = $props();

  let canvas: HTMLCanvasElement | null = $state(null);

  $effect(() => {
    const target = canvas;
    const frame = bitmap;
    if (!target || !frame) return;
    if (target.width !== frame.width) target.width = frame.width;
    if (target.height !== frame.height) target.height = frame.height;
    const ctx = target.getContext('2d');
    ctx?.putImageData(new ImageData(frame.pixels, frame.width, frame.height), 0, 0);
  });
</script>

<section class="stage" aria-labelledby="stage-title">
  <div class="stage__head">
    <h2 class="group__title" id="stage-title">{t.preview.title}</h2>
    <p class="status" class:status--busy={busy}>
      <span class="status__dot" aria-hidden="true"></span>
      <span>{statusLabel}</span>
      {#if bitmap}
        <span>· {t.preview.dimensions(bitmap.width, bitmap.height)}</span>
      {/if}
    </p>
  </div>

  <div class="stage__canvas-holder">
    {#if bitmap}
      <canvas
        bind:this={canvas}
        class="stage__canvas"
        role="img"
        aria-label={t.preview.canvasLabel(fileName ?? '')}
      ></canvas>
    {:else}
      <p class="stage__empty">{t.preview.empty}</p>
    {/if}
  </div>

  <!-- The live region is separate from the status line above so that dragging
       a slider does not announce every intermediate frame: it is written only
       when a render settles. -->
  <p class="visually-hidden" role="status" aria-live="polite">{announcement}</p>
</section>
