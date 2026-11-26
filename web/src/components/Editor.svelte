<script lang="ts">
  import { t, todayFormatted, formatNumber } from '../i18n';
  import FileDrop from './FileDrop.svelte';
  import RangeField from './RangeField.svelte';
  import PreviewStage from './PreviewStage.svelte';
  import { EngineClient, ClientError } from '../engine/client';
  import type { PreviewBitmap } from '../engine/module';
  import {
    DEFAULT_OPTIONS,
    LIMITS,
    LUMINANCE_WARN_ABOVE,
    normaliseMarkText,
    relativeLuminance,
    type MarkOptions,
    type TileDensity,
  } from '../engine/options';

  /** Longest edge of the preview bitmap. Enough for a 27" display at 2x. */
  const PREVIEW_MAX_PX = 1600;
  const DEBOUNCE_MS = 140;

  let file = $state<File | null>(null);
  let rawText = $state('');
  let options = $state<MarkOptions>({ ...DEFAULT_OPTIONS });

  let bitmap = $state<PreviewBitmap | null>(null);
  let busy = $state(false);
  let exporting = $state(false);
  let announcement = $state('');
  let problem = $state<string | null>(null);
  let unsupported = $state(false);

  const client = new EngineClient();
  let debounce: ReturnType<typeof setTimeout> | null = null;
  let generation = 0;
  /** A preview is running. The engine is synchronous: a second one would queue. */
  let inFlight = false;
  /** Something changed while one was running; render again when it finishes. */
  let restartWanted = false;

  const mark = $derived(normaliseMarkText(rawText));
  const remaining = $derived(LIMITS.textMaxChars - mark.text.length);
  const colourTooLight = $derived(relativeLuminance(options.colour) > LUMINANCE_WARN_ABOVE);
  const canExport = $derived(Boolean(file) && bitmap !== null && !exporting);

  const statusLabel = $derived(
    problem
      ? t.preview.statusFailed
      : busy
        ? t.preview.statusRendering
        : bitmap
          ? t.preview.statusReady
          : t.preview.statusIdle,
  );

  $effect(() => () => client.dispose());

  async function chooseFile(chosen: File): Promise<void> {
    problem = null;
    file = chosen;
    bitmap = null;
    const bytes = new Uint8Array(await chosen.arrayBuffer());
    client.setImage(bytes);
    requestRender();
  }

  function clearFile(): void {
    file = null;
    bitmap = null;
    problem = null;
    announcement = '';
    restartWanted = false;
    if (debounce) clearTimeout(debounce);
    client.terminate(new ClientError('internal', 'document removed'));
  }

  function scheduleRender(): void {
    if (!file) return;
    if (debounce) clearTimeout(debounce);
    debounce = setTimeout(requestRender, DEBOUNCE_MS);
  }

  /**
   * Latest wins: at most one preview in flight, and at most one more waiting.
   *
   * The debounce alone is not enough and a longer one is not the fix. The
   * engine call is synchronous, so while a render runs the worker cannot pick
   * up a newer request — anything sent meanwhile queues behind it and runs to
   * completion whether or not its result still matters. Dragging a slider
   * with pauses in it would leave the preview catching up long after the drag
   * stopped. A debounce long enough to prevent that is long enough to feel
   * broken.
   *
   * There is no parameter slot to keep because the parameters *are* the
   * component's state, read at call time, so the newest render is always the
   * one that runs next. The flag is all the slot that needs.
   */
  function requestRender(): void {
    if (!file) return;
    if (inFlight) {
      restartWanted = true;
      return;
    }
    void render();
  }

  async function render(): Promise<void> {
    if (!file) return;
    const mine = ++generation;
    inFlight = true;
    busy = true;
    announcement = '';
    try {
      const next = await client.preview(mark.text, $state.snapshot(options), PREVIEW_MAX_PX);
      if (mine !== generation) return; // a newer render has already started
      bitmap = next;
      problem = null;
      announcement = t.preview.statusAnnounce;
    } catch (error) {
      if (mine !== generation) return;
      problem = messageFor(error);
      announcement = problem;
    } finally {
      inFlight = false;
      if (restartWanted && file) {
        // Straight into the next one, without dropping `busy` in between:
        // the status line should not flicker to 'à jour' mid-drag.
        restartWanted = false;
        void render();
      } else if (mine === generation) {
        busy = false;
      }
    }
  }

  async function download(): Promise<void> {
    if (!file) return;
    exporting = true;
    problem = null;
    try {
      const pdf = await client.pdf(mark.text, $state.snapshot(options));
      const name = file.name.replace(/\.[^.]+$/, '') + t.actions.suffix + '.pdf';
      const url = URL.createObjectURL(new Blob([pdf as BlobPart], { type: 'application/pdf' }));
      const anchor = document.createElement('a');
      anchor.href = url;
      anchor.download = name;
      anchor.rel = 'noopener';
      // In the document, and revoked late: Firefox has historically ignored a
      // click on a detached anchor, and revoking in the same tick as the click
      // can cancel the download before the browser has read the blob.
      document.body.append(anchor);
      anchor.click();
      anchor.remove();
      setTimeout(() => URL.revokeObjectURL(url), 60_000);
      announcement = t.actions.downloadReady(name);
    } catch (error) {
      problem = messageFor(error);
      announcement = problem;
    } finally {
      exporting = false;
    }
  }

  function applyPreset(purpose: string, label: string): void {
    rawText = t.presetTemplate(purpose, todayFormatted());
    announcement = t.mark.presetApplied(label);
    scheduleRender();
  }

  function resetOptions(): void {
    options = { ...DEFAULT_OPTIONS };
    scheduleRender();
  }

  /**
   * One engine status, one sentence. The mapping is deliberately flat and
   * total rather than clever: every kind the client can produce is named, so
   * adding one to `ClientFailureKind` is a type error here rather than a
   * silent fall through to `generic`.
   */
  function messageFor(error: unknown): string {
    if (!(error instanceof ClientError)) return t.errors.generic;
    switch (error.kind) {
      case 'argument':
        return t.errors.refused;
      case 'decode':
        return t.errors.decode;
      case 'unsupported':
        return t.errors.unsupportedFormat;
      case 'oom':
        return t.errors.outOfMemory;
      case 'internal':
        return t.errors.crashed;
      case 'timeout':
        return t.errors.timeout;
      case 'no-image':
        return t.errors.noImage;
      case 'unsupported-browser':
        unsupported = true;
        return t.errors.unsupportedBrowser;
      default:
        return t.errors.generic;
    }
  }

  const densities: { value: TileDensity; label: string }[] = [
    { value: 'low', label: t.controls.densityLow },
    { value: 'normal', label: t.controls.densityNormal },
    { value: 'high', label: t.controls.densityHigh },
  ];
</script>

{#if unsupported}
  <p class="alert" role="alert">{t.errors.unsupportedBrowser}</p>
{/if}

<div class="editor">
  <div class="rail">
    <div class="group">
      <h2 class="group__title">{t.drop.groupTitle}</h2>
      <FileDrop
        {file}
        onfile={chooseFile}
        onclear={clearFile}
        onreject={(message) => (problem = message)}
      />
    </div>

    <div class="group">
      <h2 class="group__title" id="mark-group">{t.mark.groupTitle}</h2>

      <div class="field">
        <label class="field__label" for="mark-text">{t.mark.textLabel}</label>
        <textarea
          class="textarea"
          id="mark-text"
          rows="3"
          maxlength={LIMITS.textMaxChars}
          placeholder={t.mark.textPlaceholder}
          aria-describedby="mark-text-hint"
          bind:value={rawText}
          oninput={scheduleRender}
        ></textarea>
        <p class="field__hint" id="mark-text-hint">
          {t.mark.textHint} · {t.mark.charactersRemaining(Math.max(0, remaining))}
        </p>
        {#if mark.truncatedLines}
          <p class="warning-note">{t.mark.tooManyLines}</p>
        {/if}
      </div>

      <div class="field">
        <span class="field__label" id="presets-label">{t.mark.presetsLabel}</span>
        <div class="preset-list" role="group" aria-labelledby="presets-label">
          {#each t.presets as preset (preset.id)}
            <button
              type="button"
              class="preset"
              onclick={() => applyPreset(preset.purpose, preset.label)}
            >
              {preset.label}
            </button>
          {/each}
        </div>
        <p class="field__hint">{t.mark.presetsHint}</p>
      </div>
    </div>

    <div class="group">
      <h2 class="group__title">{t.controls.groupTitle}</h2>

      <RangeField
        id="opt-opacity"
        label={t.controls.opacity}
        value={options.opacity}
        min={LIMITS.opacity.min}
        max={LIMITS.opacity.max}
        step={LIMITS.opacity.step}
        readout={formatNumber(options.opacity)}
        valueText={`${Math.round(options.opacity * 100)} ${t.controls.percent}`}
        oninput={(value) => {
          options.opacity = value;
          scheduleRender();
        }}
      />

      <RangeField
        id="opt-angle"
        label={t.controls.angle}
        value={options.angleDeg}
        min={LIMITS.angleDeg.min}
        max={LIMITS.angleDeg.max}
        step={LIMITS.angleDeg.step}
        readout={`${options.angleDeg}${t.controls.degrees}`}
        valueText={`${options.angleDeg} degrés`}
        oninput={(value) => {
          options.angleDeg = value;
          scheduleRender();
        }}
      />

      <RangeField
        id="opt-size"
        label={t.controls.fontSize}
        hint={t.controls.fontSizeHint}
        value={options.fontSizePct}
        min={LIMITS.fontSizePct.min}
        max={LIMITS.fontSizePct.max}
        step={LIMITS.fontSizePct.step}
        readout={`${formatNumber(options.fontSizePct, 1)} ${t.controls.percent}`}
        valueText={`${formatNumber(options.fontSizePct, 1)} ${t.controls.percent}`}
        oninput={(value) => {
          options.fontSizePct = value;
          scheduleRender();
        }}
      />

      <div class="field">
        <span class="field__label" id="density-label">{t.controls.density}</span>
        <div class="segmented" role="radiogroup" aria-labelledby="density-label">
          {#each densities as density (density.value)}
            <label class="segmented__option">
              <input
                type="radio"
                name="density"
                value={density.value}
                checked={options.density === density.value}
                onchange={() => {
                  options.density = density.value;
                  scheduleRender();
                }}
              />
              <span>{density.label}</span>
            </label>
          {/each}
        </div>
      </div>

      <div class="field">
        <label class="field__label" for="opt-colour">{t.controls.colour}</label>
        <div class="colour-field">
          <input
            class="colour-input"
            type="color"
            id="opt-colour"
            value={options.colour}
            aria-describedby="colour-hint"
            oninput={(event) => {
              options.colour = event.currentTarget.value;
              scheduleRender();
            }}
          />
          <p class="field__hint" id="colour-hint">{t.controls.colourHint}</p>
        </div>
        {#if colourTooLight}
          <p class="warning-note" role="status">{t.controls.colourTooLight}</p>
        {/if}
      </div>

      <div class="actions">
        <button type="button" class="btn btn--quiet" onclick={resetOptions}>
          {t.controls.reset}
        </button>
      </div>
    </div>

    <div class="group">
      <h2 class="group__title">{t.actions.groupTitle}</h2>
      {#if problem}
        <p class="alert" role="alert">{problem}</p>
      {/if}
      <div class="actions">
        <button
          type="button"
          class="btn btn--primary"
          disabled={!canExport}
          onclick={download}
        >
          {exporting ? t.actions.preparing : t.actions.download}
        </button>
      </div>
    </div>
  </div>

  <div class="editor__stage">
    <PreviewStage
      {bitmap}
      {busy}
      {statusLabel}
      {announcement}
      fileName={file?.name ?? null}
    />
  </div>
</div>
