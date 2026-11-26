<script lang="ts">
  interface Props {
    id: string;
    label: string;
    hint?: string;
    value: number;
    min: number;
    max: number;
    step: number;
    /** What a screen reader should say instead of the bare number. */
    valueText: string;
    /** The same figure, shown beside the label. */
    readout: string;
    oninput: (value: number) => void;
  }

  let { id, label, hint, value, min, max, step, valueText, readout, oninput }: Props =
    $props();
</script>

<div class="field">
  <div class="field__row">
    <label class="field__label" for={id}>{label}</label>
    <!-- aria-hidden: the range input already announces its value, and the
         readout would otherwise be read twice. -->
    <output class="readout" for={id} aria-hidden="true">{readout}</output>
  </div>
  <input
    class="range"
    type="range"
    {id}
    {min}
    {max}
    {step}
    {value}
    aria-valuetext={valueText}
    aria-describedby={hint ? `${id}-hint` : undefined}
    oninput={(event) => oninput(event.currentTarget.valueAsNumber)}
  />
  {#if hint}
    <p class="field__hint" id="{id}-hint">{hint}</p>
  {/if}
</div>
