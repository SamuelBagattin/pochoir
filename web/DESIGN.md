# Pochoir — design system

Written before the CSS, and the CSS is built to it. If the two disagree, one of
them is wrong and this document is the one to fix first.

Every number here is either measured off a reference site (attributed) or
derived by us (marked **ours**). Nothing is "about right".

## 1. What this interface has to be

Someone is about to hand this page a photograph of their passport. The
interface has one job before it has any other: to look like it was built by
people who are careful. Sober, precise, quietly competent — a well-made
instrument, not a landing page.

Three consequences we hold to:

- **The document is the only thing on screen that gets to be interesting.**
  Everything else is chrome and recedes: no colour competing with the preview,
  no decoration beside it.
- **Restraint reads as competence.** One accent, two radii, no shadows, no
  gradients, no illustration.
- **It must survive 200% zoom and a keyboard**, because we claim RGAA AA as a
  differentiator and a claim you have not built for is a lie you have not been
  caught in yet.

## 2. Research: what was looked at, and what was taken

Studied in Chrome on 2026-11-26, with computed styles extracted from the live
DOM rather than eyeballed.

| Reference | Why it was relevant | Measured | Taken / rejected |
| --- | --- | --- | --- |
| **Tailscale** | A security product that refuses to look like one; near-monochrome | Type 12/14/16/20/32/48/64 px; tracking a flat **−0.01em** at every size; body 16/24 (1.5); display 64/70.4 (1.1); weight 500 almost everywhere; radii only 8/16/pill; the only shadow on the page is `rgba(24,23,23,0.02) 0 4px 8px` — i.e. nothing | **Taken:** −0.01em tracking above 20px, 1.5 body / 1.1 display line heights, shadows ≈ none. **Rejected:** 8/16px radii (too soft for an instrument) |
| **Stripe** | Type and spacing discipline | Ink is `rgb(6,27,49)` — **never pure black**; body is a desaturated `rgb(80,97,122)`; radii 4/6px, i.e. small and few | **Taken:** ink is never `#000`, secondary text is a real step down the ramp not an opacity, radii stay small (we use 3/6) |
| **Linear** | A dense tool UI | Grey ramp doing all the work; variable weight 510 rather than 500; 12–15px dominant sizes | **Taken:** a real neutral ramp with named steps. **Rejected:** its 12–13px density — our users are not power users on this page every day |
| **Plausible, Proton** | Privacy is their pitch, as it is ours | Both lean on purple/violet and a marketing hero | **Rejected wholesale.** Violet + big centred hero is exactly the look we were told to avoid, and both read as SaaS marketing rather than tool |
| **Resend** | Craft at the dark end | Serif display over near-black, one object | **Rejected:** beautiful, and wrong for a utility someone reaches once |
| **Squoosh** | The closest structural analogue: a single-purpose local tool with a live preview | Two-pane: canvas dominant, controls in a fixed side rail | **Taken:** the two-pane split and the controls rail. **Rejected:** the blobby pink marketing shell around it |
| **Filigrane Facile** (the tool we are replacing) | Studied once, to know precisely what to avoid | DSFR: Marianne `#000091`, the Marianne typeface, the blue/white/red block, `Ajouter le filigrane` as a centred blue button | **Avoided deliberately.** We use no blue anywhere, no state iconography, no DSFR component shapes. Looking like a government service would be dishonest — we are not one — and is a real legal risk when we compete with an actual state product |

## 3. Colour

A warm neutral ramp (hue ≈ 40°, very low chroma) plus exactly one accent. Warm
rather than cool because cool greys plus a blue accent is the government tool,
and because paper is warm.

| Token | Hex | Role |
| --- | --- | --- |
| `--c-paper` | `#FBFAF7` | Page background |
| `--c-panel` | `#FFFFFF` | Raised surface: controls rail, cards |
| `--c-sunken` | `#F1EEE9` | Recessed: the preview stage behind the document |
| `--c-inset` | `#E7E2DA` | Deeper recess: empty drop target, slider track |
| `--c-line` | `#DDD7CD` | Decorative rules and separators (not a control boundary) |
| `--c-edge` | `#8F8678` | **Control boundaries** — inputs, buttons, focusable outlines |
| `--c-ink-3` | `#6E675C` | Tertiary text: hints, units, help |
| `--c-ink-2` | `#4A443B` | Secondary text: labels, body in sidebars |
| `--c-ink-1` | `#1C1A15` | Primary text, and the primary button's fill |
| `--c-mark` | `#B23A17` | The single accent |

### Contrast, measured (WCAG 2.1 relative luminance, computed not estimated)

| Pair | Ratio | Requirement | Verdict |
| --- | --- | --- | --- |
| `ink-1` on `paper` | **16.66** | 4.5 (AA text) | pass |
| `ink-1` on `sunken` | **15.02** | 4.5 | pass |
| `ink-2` on `paper` | **9.22** | 4.5 | pass |
| `ink-3` on `paper` | **5.35** | 4.5 | pass |
| `ink-3` on `inset` | **4.33** | 4.5 | **fails — `ink-3` is never used on `inset`** |
| `mark` on `paper` | **5.73** | 4.5 | pass |
| `mark` on `panel` | **5.98** | 4.5 | pass |
| `paper` on `ink-1` (primary button) | **16.66** | 4.5 | pass |
| `edge` on `paper` | **3.44** | 3.0 (1.4.11 non-text) | pass |
| `edge` on `panel` | **3.59** | 3.0 | pass |
| `edge` on `sunken` | **3.10** | 3.0 | pass |
| `line` on `paper` | 1.37 | — | decorative only, never a control boundary |

The one failure is recorded rather than hidden: `--c-ink-3` on `--c-inset` is
4.33 and so is forbidden. Hint text sits on `paper` or `panel`, never in a
recess.

### Why `#B23A17`, and why only one accent — **ours**

It is the red-brown of a rubber stamp and of a `COPIE` overprint, which is
literally what this tool makes. It is as far from Marianne blue `#000091` as it
is from the violet-to-blue gradient every AI-generated page arrives wearing.

It is used in exactly three places: the stencil mark in the wordmark, the
watermark preview's own affordances, and error text. It is **not** the button
colour — the primary button is ink on paper, because the accent earns its
meaning by being rare.

There is no semantic green, no semantic blue, no second red. Status is carried
by words and position, never by colour alone (RGAA 3.1 / WCAG 1.4.1).

### Focus — **ours**

`outline: 2px solid var(--c-ink-1); outline-offset: 2px`. Ink, not accent:
16.66 against paper and it stays visible on the dark primary button because the
2px offset shows `paper` between fill and ring. One focus treatment for every
focusable thing on the site; `:focus-visible` only, never `:focus { outline:
none }`.

### Dark mode

Not shipped. The tokens are structured so adding a `prefers-color-scheme` block
is a token swap rather than a rewrite, but shipping two themes means verifying
contrast twice and we would rather verify one properly. Recorded as a decision,
not an oversight.

## 4. Type

**No webfont.** Zero bytes, nothing third-party, nothing to self-host, no
licence file, and no ADR needed to depart from ADR-0001. The character comes
from the pairing instead: **prose in the system sans, every label, numeric
readout and unit in the system mono.** That is the single most distinguishing
decision in this document and it costs nothing — it also reads as *instrument*
rather than *landing page*, which is the whole brief.

```
--font-sans: ui-sans-serif, system-ui, -apple-system, "Segoe UI", Roboto,
             "Helvetica Neue", Arial, sans-serif;
--font-mono: ui-monospace, SFMono-Regular, "SF Mono", Menlo, Consolas,
             "Liberation Mono", monospace;
```

### Scale — **ours**, informed by Tailscale's measured steps

Rounded ratio ≈ 1.25, truncated to seven steps because a page with eleven text
sizes has no hierarchy.

| Token | Size | Line height | Tracking | Use |
| --- | --- | --- | --- | --- |
| `--t-display` | 40px | 44px (1.1) | −0.01em | Page title, once per page |
| `--t-h2` | 26px | 32px (1.23) | −0.01em | Section heading |
| `--t-h3` | 19px | 26px (1.37) | −0.005em | Sub-section, panel title |
| `--t-body` | 16px | 25px (1.56) | 0 | Prose. Never smaller for prose |
| `--t-sm` | 14px | 21px (1.5) | 0 | Secondary prose, help text |
| `--t-label` | 13px | 16px | +0.02em | Mono. Control labels |
| `--t-micro` | 12px | 16px | +0.04em | Mono, uppercase. Units, metadata |

All sizes are declared in `rem` against a 16px root so browser font-size
settings scale them; the px figures above are the rendered result at default
settings. Prose never goes below 16px — a deliberate departure from Linear's
12–13px density.

Weights: 400 for prose, 600 for headings and labels. Two weights, no more; 500
is invisible against 400 in most system stacks and pretending otherwise is how
hierarchy dissolves.

Measure: prose is capped at `68ch`. The explainer page is a single column, not
a grid.

## 5. Space

4px base. Nine steps, named by size so nobody has to count:

```
--s-1: 4px   --s-2: 8px   --s-3: 12px  --s-4: 16px  --s-5: 24px
--s-6: 32px  --s-7: 48px  --s-8: 64px  --s-9: 96px
```

Rules, so that spacing is never arbitrary:

- Label to its control: `--s-2`.
- Between controls in a group: `--s-4`.
- Between control groups: `--s-5`, plus a `--c-line` rule.
- Section to section on a page: `--s-7`.
- Page gutters: `--s-5` under 720px, `--s-6` above.
- Nothing outside the scale. If a thing needs 18px, the thing is wrong.

## 6. Shape

**Radius policy — two values, no exceptions.**

```
--r-1: 3px   /* controls: inputs, buttons, swatches */
--r-2: 6px   /* containers: panels, the preview stage, the drop target */
```

Sharp, because precision instruments are sharp. Explicitly banned: pill
buttons, `border-radius: 16px` cards, circular avatars — the vocabulary of the
SaaS dashboard we are not.

**Shadow policy — none.**

Not "subtle". None. Depth is carried by 1px `--c-edge` / `--c-line` borders and
by the `paper → panel → sunken → inset` background steps, which is why that
ramp has four values. The only non-border depth cue in the build is the
drop-target's `inset` ring while a file is dragged over it, and that is a
border change, not a shadow.

**Borders:** 1px everywhere. A control that is interactive gets `--c-edge`
(≥3:1). A line that only separates gets `--c-line`.

## 7. Density and alignment

- **The editor is two panes above 960px:** a controls rail of a fixed `360px`
  and the preview taking the rest. Below 960px they stack, controls first —
  because on a phone you have already chosen the file and the next thing you do
  is type the mark text.
- **The rail is the only scrolling column** on wide screens; the preview is
  sticky, because watching the mark change while dragging a slider is the
  entire product claim and it cannot be below the fold.
- Controls are **label above input**, full width of the rail, left-aligned. No
  two-column forms: they break at 200% zoom and they read as a settings dialog.
- Every box aligns to the 4px grid. Control height is 40px (`--s-4` × 2.5),
  touch targets never below 40×40 — above the 24×24 of WCAG 2.5.8, below the
  44px of 2.5.5 AAA, which we do not claim.
- At 200% zoom the two panes collapse to one at the same 960px CSS breakpoint,
  which is what makes 1.4.10 reflow hold without a separate mobile layout.

## 8. Motion

Transitions only on `background-color`, `border-color` and `outline-color`, at
`120ms ease`. Nothing moves position, nothing fades in on scroll, nothing
animates on load. All of it inside
`@media (prefers-reduced-motion: reduce) { * { transition-duration: 0.01ms } }`.

## 9. The list of things this design must never grow

Taken from the brief, kept here because a list of banned things is more useful
to the next person than a mood board:

purple-to-blue gradients · a hero with enormous centred type and a vague
tagline · three-column feature grids with generic line icons · rounded cards
floating on a grey wash · soft shadows · emoji in headings · glassmorphism ·
"✨" · arbitrary spacing · filler copy · and anything that would make a worried
person reasonably mistake us for the government.
