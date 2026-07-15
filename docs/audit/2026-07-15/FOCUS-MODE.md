# Focus Mode characterization

The protected behavior is the existing reading interaction, not a redesign. The characterization suite covers:

- regular block centering on the focus anchor;
- long blocks remaining under native scrolling when the anchor is already inside;
- long-block entry from above and return from below using readable edge buffers;
- Chinese and mixed-language sentence ranges, quote attachment, no-gap coverage, and whitespace filtering;
- navigation generation rejection for an older response and for a same-generation path change.

Focused run on 2026-07-15: 3 Vitest files, 13 tests passed. The work in this audit did not change Focus Mode scroll math, typography, colors, or viewport-anchor code. The only CSS cleanup removed two keyframe declarations with no animation consumer (`slideIn` and `task-row-in`), outside the Focus Mode behavior.

The browser QA reader script now requires `IMMERSIVE_QA_MANIFEST` instead of silently reading a real user Library manifest. A future live run must supply a manifest under the isolated `IMMERSIVE_QA_RUN_ID` root.
