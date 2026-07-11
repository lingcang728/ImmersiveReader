# 沉浸阅读 — Design System

**Reading this as:** desktop product UI for a single power reader, with an editorial night-study language, leaning toward native CSS + hybrid Chinese serif (body) / sans (chrome) — a crafted reading instrument, not SaaS.

## Dials

| Dial | Value | Why |
|------|-------|-----|
| `DESIGN_VARIANCE` | 4 | Calm, readable structure; subtle offset in resume card only |
| `MOTION_INTENSITY` | 2 | Hover/active only; motion would compete with long-form attention |
| `VISUAL_DENSITY` | 3 | Gallery-air for night reading; chrome recedes |

## Direction: MMbook monochrome shell + blue link accent

**Product chrome (bookshelf, top bar, menus)** follows the active MMbook theme tokens (`--bg`, `--bg-secondary`, `--text`, `--heading`, `--hr` from `apps/desktop/src/lib/theme/themes.ts`). Neutrals carry most of the surface.

**Accent** is the theme **link blue** (`--link` / `--link-hover`, e.g. suzhi-dark `#7ba4d4`). Use for focus rings, badges, secondary actions, and progress fills. **No gold / brass / lamp yellow** on the production shelf.

**Reading body** keeps MMbook theme variables. Spotlight and long-form typography stay on the existing reading path.

Legacy “夜灯” gold lives only in `docs/design/prototype` as historical reference.

## Tokens

### Color

| Token | Value | Role |
|-------|-------|------|
| `--bg-void` | `#100f0d` | Title bar / deepest void |
| `--bg-base` | `#161412` | App canvas |
| `--bg-raised` | `#1c1916` | Cards, panels |
| `--bg-elevated` | `#23201c` | Overlay panels, hover lift |
| `--text-primary` | `#e4ddd0` | Body & titles (off-white, warm) |
| `--text-secondary` | `#a39c8f` | Supporting |
| `--text-tertiary` | `#6f695f` | Nonessential decoration only |
| `--text-faint` | `#4d4942` | Disabled or purely decorative hints only |
| `--accent` | `#c4a46a` | Single accent (lantern) |

No pure `#000` / `#fff`. One accent locked across all three screens.

Small readable metadata and keyboard instructions use `--text-secondary`; tertiary and faint values are never used for required 12–14px information.

### Typography

| Role | Family | Notes |
|------|--------|-------|
| UI chrome | Noto Sans SC | Labels, badges, buttons |
| Reading body | Noto Serif SC | Long-form Chinese, 18px / 1.9 lh |
| Numerals | Tabular / mono | Progress %, article index |

Measure: `~38em` centered column. Paragraph spacing `1.65em`.

### Radius

`6 / 10 / 14 / 18` — one soft system, never mixed with full-pill cards.

### Spacing

4px base rhythm: 4 · 8 · 12 · 16 · 24 · 32 · 48 · 64.

## Screens (1440×900)

1. **书架** (`index.html`) — single entry: resume hero, collection grid, dual open modes (精读 primary / 连读 whisper), 临时内容.
2. **连读** (`reading.html`) — flow column + TOC command palette open + edge progress rail + next-article seam.
3. **聚光灯** (`focus.html`) — same article; graduated dim/blur; chrome gone; hair-thin rail.

## Dual action pattern

- **精读**: solid accent button (in-app)
- **连读**: ghost text + external arrow (browser flow)

Not two equal buttons.

## Explicitly avoided

AI purple gradients, decorative glass, emoji icons, stat widgets, equal-weight button rows, pure black/white, English lorem.

## Challenge result and real states

The three archive-room challenge drafts live in `docs/design/challenges`. Night Lamp remains the final direction; only its clearer brass overlay boundary and shorter spotlight hint were retained.

Production surfaces include loading, empty library, corrupt manifest, unwritable library, missing chapter, missing production tool, and browser-local progress states. Prototype-only window controls and bottom screen navigation are not part of the product.
