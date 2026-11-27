<script lang="ts">
  import { t, todayFormatted, formatNumber, formatCharacterList } from '../i18n';
  import FileDrop from './FileDrop.svelte';
  import RangeField from './RangeField.svelte';
  import PreviewStage from './PreviewStage.svelte';
  import { EngineClient, ClientError } from '../engine/client';
  import { NO_WARNINGS, type MarkWarnings, type PreviewBitmap } from '../engine/module';
  import {
    AUTOMATIC_COLOUR,
    DEFAULT_OPTIONS,
    LIMITS,
    normaliseMarkText,
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
  /**
   * A failure that is specifically about the mark text, shown under the
   * textarea rather than in the export group. Statuses 6 and 7 are the only
   * two the engine attributes to the text, and they are the two whose fix is
   * in that field — which at 200 % zoom is not on the same screen as the
   * export button.
   */
  let markProblem = $state<string | null>(null);
  let unsupported = $state(false);
  /**
   * What the engine said about the mark it drew. Not failures: the document
   * exists either way, and these are the two facts about it that the pixels
   * do not show — so if we do not say them, nobody does. The CLI always has.
   */
  let warnings = $state<MarkWarnings>(NO_WARNINGS);
  /**
   * The colour kept aside while the mode is `Automatique`, so that switching
   * back does not silently hand the user a different colour from the one they
   * picked. `<input type="color">` has no empty state to return to.
   */
  let customColour = $state<`#${string}`>('#1a1a1a');

  const client = new EngineClient();
  let debounce: ReturnType<typeof setTimeout> | null = null;
  let generation = 0;
  /** A preview is running. The engine is synchronous: a second one would queue. */
  let inFlight = false;
  /** Something changed while one was running; render again when it finishes. */
  let restartWanted = false;

  const mark = $derived(normaliseMarkText(rawText));
  const remaining = $derived(LIMITS.textMaxChars - mark.text.length);
  const canExport = $derived(Boolean(file) && bitmap !== null && !exporting);
  const colourIsAutomatic = $derived(options.colour === AUTOMATIC_COLOUR);

  /**
   * The contrast warning is one bit on the wire and two sentences here,
   * because the remedy differs and we are the side that knows which request
   * we sent. On `Automatique` the page defeats both greys and there is nothing
   * to correct; on a chosen colour there is.
   */
  const contrastWarning = $derived(
    !warnings.poorContrast
      ? null
      : colourIsAutomatic
        ? t.warnings.poorContrastAuto
        : t.warnings.poorContrastFixed,
  );
  const shrinkWarning = $derived(warnings.shrunk ? t.warnings.markShrunk : null);

  const statusLabel = $derived(
    problem || markProblem
      ? t.preview.statusFailed
      : busy
        ? t.preview.statusRendering
        : bitmap
          ? t.preview.statusReady
          : file
            ? t.preview.statusAwaitingMark
            : t.preview.statusIdle,
  );

  /**
   * What the stage says when there is no bitmap. Two different reasons to
   * have none, and they want different sentences: no document at all, or a
   * document with nothing to draw on it yet.
   */
  const stageEmptyMessage = $derived(file ? t.preview.awaitingMark : t.preview.empty);

  $effect(() => () => client.dispose());

  async function chooseFile(chosen: File): Promise<void> {
    problem = null;
    markProblem = null;
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
    markProblem = null;
    warnings = NO_WARNINGS;
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

    // Never ask the engine for something it has already decided is invalid.
    // `Mark::prepared()` refuses an empty or whitespace-only mark with
    // BadArgument, so rendering here would report our own defect to somebody
    // who has done nothing wrong — and it would do it on every single load,
    // because a document is always chosen before any text is typed. An image
    // with no mark yet is a perfectly good state; it is just not a preview.
    if (mark.isBlank) {
      generation += 1; // discard anything already in flight
      restartWanted = false;
      bitmap = null;
      problem = null;
      markProblem = null;
      warnings = NO_WARNINGS;
      busy = false;
      announcement = '';
      return;
    }

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
      markProblem = null;
      warnings = next.warnings;
      announcement = t.preview.statusAnnounce;
    } catch (error) {
      if (mine !== generation) return;
      announcement = report(error);
    } finally {
      inFlight = false;
      if (restartWanted && file) {
        // Straight into the next one, without dropping `busy` in between:
        // the status line should not flicker to 'à jour' mid-drag. Through
        // `requestRender` rather than `render`, so that text deleted while a
        // render was running is caught by the blank guard above.
        restartWanted = false;
        void requestRender();
      } else if (mine === generation) {
        busy = false;
      }
    }
  }

  async function download(): Promise<void> {
    if (!file) return;
    exporting = true;
    problem = null;
    markProblem = null;
    try {
      const pdf = await client.pdf(mark.text, $state.snapshot(options));
      // The document's own warnings, not the preview's. They agree in every
      // case measured, and if they ever did not it is the document that was
      // downloaded and the document the user is about to send.
      warnings = pdf.warnings;
      const name = file.name.replace(/\.[^.]+$/, '') + t.actions.suffix + '.pdf';
      const url = URL.createObjectURL(
        new Blob([pdf.bytes as BlobPart], { type: 'application/pdf' }),
      );
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
      announcement = report(error);
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

  function chooseColourMode(automatic: boolean): void {
    options.colour = automatic ? AUTOMATIC_COLOUR : customColour;
    scheduleRender();
  }

  /**
   * Put a failure where the user can act on it, and return what to announce.
   *
   * The mapping is deliberately flat and total rather than clever: every kind
   * the client can produce is named, so adding one to `ClientFailureKind` is a
   * type error here rather than a silent fall through to `generic`.
   */
  function report(error: unknown): string {
    const { message, onMarkText } = describe(error);
    if (onMarkText) {
      markProblem = message;
      problem = null;
    } else {
      problem = message;
      markProblem = null;
    }
    return message;
  }

  function describe(error: unknown): { message: string; onMarkText: boolean } {
    if (!(error instanceof ClientError)) {
      return { message: t.errors.generic, onMarkText: false };
    }
    switch (error.kind) {
      case 'unsupported-character': {
        // The payload is the offending characters themselves — data, never a
        // sentence. The French around them is ours; the engine's is not shown.
        if (!error.detail) {
          return { message: t.errors.unsupportedCharactersUnnamed, onMarkText: true };
        }
        const { list, count } = formatCharacterList(error.detail);
        return { message: t.errors.unsupportedCharacters(list, count), onMarkText: true };
      }
      case 'does-not-fit':
        return { message: t.errors.textDoesNotFit, onMarkText: true };
      case 'argument':
        return { message: t.errors.refused, onMarkText: false };
      case 'decode':
        return { message: t.errors.decode, onMarkText: false };
      case 'unsupported':
        return { message: t.errors.unsupportedFormat, onMarkText: false };
      case 'oom':
        return { message: t.errors.outOfMemory, onMarkText: false };
      case 'internal':
        return { message: t.errors.crashed, onMarkText: false };
      case 'timeout':
        return { message: t.errors.timeout, onMarkText: false };
      case 'no-image':
        return { message: t.errors.noImage, onMarkText: false };
      case 'unsupported-browser':
        unsupported = true;
        return { message: t.errors.unsupportedBrowser, onMarkText: false };
      default:
        return { message: t.errors.generic, onMarkText: false };
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
          aria-invalid={markProblem ? 'true' : undefined}
          aria-describedby={markProblem ? 'mark-text-hint mark-text-error' : 'mark-text-hint'}
          bind:value={rawText}
          oninput={scheduleRender}
        ></textarea>
        <p class="field__hint" id="mark-text-hint">
          {t.mark.textHint} · {t.mark.charactersRemaining(Math.max(0, remaining))}
        </p>
        {#if markProblem}
          <!-- No role="alert": it is already in the textarea's
               aria-describedby, and the preview's live region announces it
               once. Two announcements of the same sentence is worse than
               none. -->
          <p class="warning-note" id="mark-text-error">{markProblem}</p>
        {/if}
        {#if mark.truncatedLines}
          <p class="warning-note">{t.mark.tooManyLines}</p>
        {/if}
        <!-- The shrink warning belongs to the text, not to the export: this
             is the control that fixes it, and at 200 % zoom the export group
             is not on the same screen. ADR-0008's open question 4 could not
             reach here until the result header carried a warning channel. -->
        {#if shrinkWarning}
          <p class="warning-note" role="status">{shrinkWarning}</p>
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
        <span class="field__label" id="colour-label">{t.controls.colour}</span>
        <div class="segmented" role="radiogroup" aria-labelledby="colour-label">
          <label class="segmented__option">
            <input
              type="radio"
              name="colour-mode"
              value="auto"
              checked={colourIsAutomatic}
              onchange={() => chooseColourMode(true)}
            />
            <span>{t.controls.colourAuto}</span>
          </label>
          <label class="segmented__option">
            <input
              type="radio"
              name="colour-mode"
              value="fixed"
              checked={!colourIsAutomatic}
              onchange={() => chooseColourMode(false)}
            />
            <span>{t.controls.colourCustom}</span>
          </label>
        </div>

        {#if colourIsAutomatic}
          <p class="field__hint" id="colour-hint">{t.controls.colourAutoHint}</p>
        {:else}
          <div class="colour-field">
            <input
              class="colour-input"
              type="color"
              id="opt-colour"
              aria-label={t.controls.colourPickerLabel}
              value={customColour}
              aria-describedby="colour-hint"
              oninput={(event) => {
                // `<input type="color">` always yields `#rrggbb`, which is the
                // only shape `colourWord` accepts; it refuses anything else
                // rather than quietly substituting a default.
                customColour = event.currentTarget.value as `#${string}`;
                options.colour = customColour;
                scheduleRender();
              }}
            />
            <p class="field__hint" id="colour-hint">{t.controls.colourHint}</p>
          </div>
        {/if}

        {#if contrastWarning}
          <p class="warning-note" role="status">{contrastWarning}</p>
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
      emptyMessage={stageEmptyMessage}
      fileName={file?.name ?? null}
    />
  </div>
</div>
