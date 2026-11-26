<script lang="ts">
  import { t, formatBytes } from '../i18n';
  import { ACCEPTED_TYPES, MAX_INPUT_BYTES } from '../engine/options';

  interface Props {
    file: File | null;
    onfile: (file: File) => void;
    onclear: () => void;
    onreject: (message: string) => void;
  }

  let { file, onfile, onclear, onreject }: Props = $props();

  let over = $state(false);
  let input: HTMLInputElement | null = $state(null);

  // Thirty lines of `dragover`/`drop`, as ADR-0001 says. No library.
  function accept(list: FileList | null | undefined): void {
    if (!list || list.length === 0) return;
    if (list.length > 1) onreject(t.drop.rejectedMultiple);
    const candidate = list[0]!;
    if (!(ACCEPTED_TYPES as readonly string[]).includes(candidate.type)) {
      onreject(t.drop.rejectedType);
      return;
    }
    if (candidate.size > MAX_INPUT_BYTES) {
      onreject(t.drop.rejectedSize);
      return;
    }
    onfile(candidate);
  }

  function onDragOver(event: DragEvent): void {
    event.preventDefault();
    over = true;
  }

  function onDrop(event: DragEvent): void {
    event.preventDefault();
    over = false;
    accept(event.dataTransfer?.files);
  }
</script>

<!-- One wrapper around the input and both of its presentations, so the
     focus ring follows the keyboard onto a control that is deliberately
     invisible. Without it, `:focus-within` has nothing to match. -->
<div class="dropzone">
{#if file}
  <div class="file-summary">
    <span class="file-summary__name">{file.name}</span>
    <span class="file-summary__meta">{formatBytes(file.size)}</span>
    <div class="actions">
      <!-- A real <label> for a real <input type="file">: the picker opens from
           the keyboard with Enter or Space because the input is focusable, not
           because of a click handler we wrote. -->
      <label class="btn drop__pick" for="pochoir-file">{t.drop.change}</label>
      <button type="button" class="btn btn--quiet" onclick={onclear}>
        {t.drop.remove}
      </button>
    </div>
  </div>
{:else}
  <div
    class="drop"
    class:drop--over={over}
    ondragover={onDragOver}
    ondragenter={onDragOver}
    ondragleave={() => (over = false)}
    ondrop={onDrop}
    role="group"
    aria-labelledby="drop-prompt"
  >
    <svg
      width="28"
      height="28"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      stroke-width="1.5"
      aria-hidden="true"
    >
      <path d="M4 15.5V19a1 1 0 0 0 1 1h14a1 1 0 0 0 1-1v-3.5" />
      <path d="M12 3.5v11" />
      <path d="m7.5 10 4.5 4.5 4.5-4.5" />
    </svg>
    <p class="drop__prompt" id="drop-prompt">
      {over ? t.drop.dropping : t.drop.prompt}
    </p>
    <p class="drop__hint" id="drop-hint">{t.drop.hint}</p>
    <label class="btn drop__pick" for="pochoir-file">{t.drop.pick}</label>
  </div>
{/if}

<input
    bind:this={input}
    class="visually-hidden"
    type="file"
    id="pochoir-file"
    accept={ACCEPTED_TYPES.join(',')}
    aria-label={t.drop.inputLabel}
    aria-describedby={file ? undefined : 'drop-hint'}
    onchange={(event) => {
      accept(event.currentTarget.files);
      // Let the same file be chosen twice in a row.
      if (input) input.value = '';
    }}
  />
</div>
