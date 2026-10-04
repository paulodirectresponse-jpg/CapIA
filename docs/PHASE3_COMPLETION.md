# CapIA — PHASE 3 COMPLETION SPEC

**Purpose:** Finish the entire Phase 3 — Editor (manual, no AI) in one autonomous goal.

**Base branch:** `claude/phase2-completion`  
**Base commit:** `84e778a77843cde2bce03e521e5dd2e7520c6cfd`  
**Target branch:** `claude/phase3-editor`  
**Entry gate:** OD-1 closed by ADR-069  
**Preview presenter:** P2 — SharedBuffer → canvas WebGL  
**Current status:** Phase 2 complete; Phase 3 allowed to start.

---

## 1. Mission

Build the complete manual CapIA editor on top of the finished headless engine.

The result must be a usable desktop video editor that allows a human editor to:

- create/open a project;
- import media;
- organize sequences;
- drag media to the timeline;
- edit clips manually;
- preview smoothly;
- work with nested sequences;
- add text/captions/transitions;
- edit audio;
- inspect properties/keyframes;
- undo/redo;
- relink media;
- export one or more deliverables.

This phase must not add AI editing, provider integrations, autonomous orchestration, reference analysis, transcription AI, or any Phase 4+ capability.

---

# 2. Delivery strategy — speed first without sacrificing the engine

The previous phases used very high rigor because they established the data model, persistence and render semantics.

For Phase 3:

## Keep maximum rigor for
- Command Engine boundaries;
- persistence;
- preview/render correctness;
- export;
- data loss/crash risks;
- security;
- media handling;
- command atomicity.

## Use pragmatic rigor for UI
- prefer E2E tests over large unit/property suites;
- no ADR for minor styling/layout decisions;
- no mutation testing for trivial UI code;
- do not block progress on pixel-perfect polish;
- fix critical usability issues, not every cosmetic imperfection;
- implement vertical slices early.

The priority is to reach a usable editor quickly.

---

# 3. Mandatory reading before implementation

Read:

- `CLAUDE.md`
- `docs/STATUS.md`
- `docs/ROADMAP.md`
- `docs/TIMELINE_UX.md`
- `docs/PREVIEW_RENDER.md`
- `docs/ARCHITECTURE.md`
- `docs/DATA_MODEL.md`
- `docs/COMMAND_SYSTEM.md`
- `docs/TEST_STRATEGY.md`
- ADR-069
- UI/Tauri code already present under `apps/desktop`
- all current frontend packages

Do not redesign existing core contracts without a compelling incompatibility.

---

# 4. Branch and workflow

Create:

`claude/phase3-editor`

Do not open a PR.

Work autonomously:

analyse → implement → test → run → inspect → fix → commit → push → CI → continue.

Do not ask the user for ordinary technical approval.

When a visual decision is unspecified:
- follow `TIMELINE_UX.md`;
- use a clean, compact, professional desktop-editor layout;
- keep components editable/refactorable;
- avoid visual imitation/copying of any third-party product.

---

# 5. Application shell

Build the real editor shell.

Required layout:

- top bar;
- left navigation/rail;
- project/media area;
- central preview;
- right inspector;
- sequence tabs;
- bottom timeline.

Panels must be:
- resizable;
- collapsible where useful;
- persisted per workspace where practical.

The editor should open into a usable default layout without configuration.

---

# 6. Design system

Create/finish a lightweight internal design system.

At minimum:
- spacing scale;
- typography;
- surface hierarchy;
- buttons;
- icon buttons;
- tabs;
- inputs;
- sliders;
- menus;
- popovers;
- tooltips;
- dialogs;
- toast/status feedback;
- empty/loading/error states.

Requirements:
- consistent dark theme;
- contrast AA where practical;
- keyboard focus visible;
- no hardcoded inconsistent one-off styling throughout the app.

Do not spend disproportionate time on visual polish before the editor works end to end.

---

# 7. Project panel

Implement the Project tree as the organizational source of truth.

Required:
- folders;
- sequences;
- nested usage count;
- sequence format badge;
- export status where available;
- inline rename;
- drag/drop organization;
- context menu;
- reveal active sequence;
- open sequence on double-click;
- create new sequence;
- duplicate sequence;
- delete sequence with appropriate validation.

Sequence tabs must be navigation only, not storage.

---

# 8. Sequence tabs

Implement:
- open multiple sequences;
- active sequence;
- close tab without deleting;
- `+` new sequence;
- inline rename on creation;
- duplicate current;
- new from preset:
  - 9:16
  - 1:1
  - 4:5
  - 16:9

Nested navigation:
- double-click nested clip opens child;
- show breadcrumb;
- indicate shared master usage;
- expose make unique.

---

# 9. Media library

Create a usable media bin/library.

Required:
- import video;
- import audio;
- import image;
- background import state;
- thumbnail;
- duration;
- type;
- online/offline/modified state;
- search/filter;
- basic sorting;
- reveal/relink actions;
- drag item to timeline.

Do not duplicate backend media state in frontend-only authority.

---

# 10. Drag and drop to timeline

Media drag/drop must produce real Command Engine operations.

Required behaviors:
- drop on main/magnetic track → insert/ripple according to engine semantics;
- drop on overlay area → place there;
- if overlay placement requires a new track, create/use one predictably;
- audio to audio track;
- image duration uses editor default, not file duration;
- invalid drop shows clear feedback.

All mutations must go through commands/transactions.

---

# 11. `ui-timeline`

Build/use a dedicated package for timeline rendering.

Suggested:

`packages/ui-timeline`

Architecture:
- canvas or WebGL renderer;
- interaction layer;
- read-model adapter;
- no DOM node per clip;
- virtualization by visible time range and visible tracks.

Render:
- ruler;
- playhead;
- tracks;
- clips;
- thumbnails;
- waveform;
- text/caption clips;
- nested clips;
- transition markers;
- keyframes;
- selection;
- markers;
- offline badges.

---

# 12. Timeline performance

Meet `TIMELINE_UX.md` §6 as closely as measurable in CI/dev environment.

Targets:
- 60 fps scroll/zoom with large timelines;
- 5,000-clip stress project;
- drag feedback <16 ms/frame;
- command commit <30 ms when feasible;
- thumbnails visible quickly;
- waveform zoom without recomputation;
- scrub response <100 ms with proxy/cache path;
- undo/redo <50 ms.

Use instrumentation.

Do not fake numbers.

If a local hardware criterion cannot be measured in CI, ship a local benchmark command and document the result as pending hardware validation.

---

# 13. Selection

Implement:
- click select;
- shift-add/remove;
- marquee selection;
- selected track;
- multi-clip state;
- clear selection;
- select clip under playhead when appropriate.

Selection state may be UI state, but edits resulting from it must be commands.

---

# 14. Move and magnetic reorder

Implement clip movement through existing placement/reorder semantics.

Required:
- overlay move;
- vertical move between tracks;
- main magnetic reorder;
- snapping;
- conflict feedback;
- ghost preview;
- commit only on drop.

If moving to/from magnetic tracks requires missing engine support, add the minimum command semantics needed and test them properly.

Do not directly mutate the document from UI.

---

# 15. Trim

Implement edge trim.

Required:
- left/right handles;
- live ghost preview;
- source limits;
- magnetic ripple;
- overlay neighbor constraints;
- snapping;
- command committed on release;
- Esc cancels gesture.

---

# 16. Split and delete

Implement:
- split at playhead;
- delete;
- ripple delete;
- trim left to playhead;
- trim right to playhead.

Default shortcuts from `TIMELINE_UX.md`:
- Ctrl+B / S
- Del
- Shift+Del
- Q
- W

Allow keymap customization later in this phase.

---

# 17. Snapping

Wire the existing pure snapping engine into the UI.

Targets:
- playhead;
- clip edges;
- markers;
- sequence start;
- keyframes;
- future transcript word boundaries when they exist.

Visual:
- snap line;
- snap target indication.

Threshold:
- pixel-based converted to Ticks at current zoom.

Do not duplicate snapping math with a divergent JS implementation.

---

# 18. Zoom and navigation

Implement:
- Ctrl+wheel horizontal zoom;
- +/-;
- slider;
- fit timeline (`Shift+Z`);
- scroll tracks vertically;
- zoom anchored to cursor/playhead;
- ruler adapting to zoom;
- timecode display.

---

# 19. Copy/paste/duplicate

Implement:
- Ctrl+C
- Ctrl+V
- Ctrl+D
- Alt+drag duplicate

Paste at playhead/selected track according to documented behavior.

Cross-sequence copy/paste required.

Cross-project copy/paste may use a serialized clipboard payload and import missing assets where safely supported.

---

# 20. Groups

Implement:
- group selected clips;
- ungroup;
- visual group indication;
- group move.

Shortcuts:
- Ctrl+G
- Ctrl+Shift+G

Use engine semantics; add minimal missing command support if necessary.

---

# 21. Tracks

Track header must expose:
- name;
- role;
- lock;
- hide;
- mute;
- solo;
- magnetic;
- resize height.

Implement:
- add track;
- delete track;
- reorder if model supports or add necessary command;
- state persistence.

Lock/mute/hide/solo must have actual behavioral effect.

---

# 22. Playback controls

Implement:
- Space play/pause;
- J/K/L shuttle;
- frame step left/right;
- Shift+left/right 10 frames;
- Home/End;
- playhead click/drag;
- loop range if already supported, otherwise omit.

Playback clock should respect `PREVIEW_RENDER.md`.

---

# 23. P2 preview presenter

Implement ADR-069 presenter in the real desktop app.

Path:

wgpu/render result
→ readback
→ WebView2 SharedBuffer
→ WebGL canvas

Required:
- 540p and 720p preview modes;
- Auto quality;
- dropped-slot metric;
- resize;
- fullscreen;
- HTML overlays;
- safe areas;
- transform overlays;
- no airspace dependency;
- proxy toggle;
- cold/warm cache behavior.

Do not reintroduce P1.

---

# 24. Residual ADR-069 validation

When environment permits, run:

`tools/s1-preview-spike/run.ps1 -P2Res 1280x720`

on real GPU.

Residual acceptance:
- CPU pipeline ≤ documented criterion;
- lost slots within ADR-069 threshold;
- no resize/DPI artifact.

If the autonomous environment cannot run a real GPU:
- do not block engineering work;
- prepare a one-click local acceptance command;
- mark residual as human/hardware acceptance pending.

Do not reopen OD-1 unless the documented trigger occurs.

---

# 25. Preview overlays

Implement editor-only overlays:
- safe areas;
- bounding box;
- resize handles where supported;
- selected clip indication;
- center/alignment guides.

Overlays must not alter render/export output.

---

# 26. Inspector

Inspector must react to current selection.

Contexts:
- video/image clip;
- audio clip;
- text;
- caption;
- transition;
- sequence.

Initial tabs:
- Basic;
- Animation/Keyframes;
- Audio;
- Speed where relevant.

Do not display controls with no functional backend semantics.

---

# 27. Transform properties

Expose and edit:
- position;
- scale;
- opacity;
- crop if supported;
- rotation if implemented in this phase.

Live preview allowed locally.

Final mutation must be a command.

---

# 28. Keyframes

Implement UI for existing keyframe engine.

Required:
- add;
- move;
- delete;
- interpolation selection;
- visual diamonds;
- property linkage;
- timeline/inspector synchronization.

Do not create a second independent keyframe model in frontend.

---

# 29. Text engine

Phase 3 must add manual text clips.

Required:
- text entity/clip support;
- deterministic render path;
- font family;
- font size;
- weight where supported;
- alignment;
- color;
- position/scale/opacity;
- keyframes on relevant properties.

Optional if time allows:
- background;
- stroke;
- shadow.

Use one render implementation for preview/export.

Document font resolution and missing-font behavior.

---

# 30. Manual captions

Captions are manual in Phase 3.

Required:
- caption track/clip representation;
- edit text;
- timing;
- split/merge where practical;
- style preset;
- preview/export parity.

Automatic transcription belongs to Phase 4.

---

# 31. Transitions

Implement a small useful initial transition set.

Minimum:
- cut (implicit);
- cross dissolve;
- fade;
- one directional/simple transform transition if architecture permits.

Transition:
- exists at a cut;
- duration editable;
- visible in timeline;
- rendered identically in preview/export.

Do not create a huge transition library.

---

# 32. Effects

Do not build a full effects marketplace/system in this phase.

Only implement any minimal registry/architecture necessary for:
- transition rendering;
- basic transform/opacity;
- future extension.

A large creative effect library is not required for Phase 3 completion.

---

# 33. Audio UI

Implement:
- waveform;
- mute/solo;
- clip volume;
- fades;
- detach audio;
- audio-only clips;
- music/SFX tracks.

Where volume automation/keyframes are supported, expose them.

---

# 34. Detach audio

Implement command semantics if missing.

Video with linked audio:
- detach creates/uses audio clip;
- timing remains aligned;
- undo restores prior state;
- no duplication/loss.

---

# 35. History

Expose visible command history.

Required:
- transaction label;
- actor;
- timestamp where available;
- undo;
- redo;
- current cursor.

Do not bypass existing durable history.

---

# 36. Offline and relink UI

When media is offline:
- visible clip badge;
- media-bin status;
- relink action;
- folder relink;
- force relink explicit and dangerous;
- conflicts shown clearly.

No silent force relink.

---

# 37. Export UI

Implement editor export flow.

Required:
- select sequence;
- preset;
- resolution;
- frame rate policy;
- encoder capability;
- destination;
- progress;
- cancel;
- validation result.

Use existing atomic export path.

---

# 38. Deliverables and batch export

Implement lightweight deliverables.

A deliverable references:
- sequence;
- preset;
- output path/name.

Required:
- create;
- remove;
- batch export;
- progress;
- per-item result.

Do not introduce Phase 5 variant orchestration here.

---

# 39. OUTPUT-H264 gate

Before Phase 3 is considered delivered:

- Windows export must have a reliable H.264 path;
- use approved capability abstraction;
- Media Foundation/hardware paths allowed;
- no silent x264/x265 GPL fallback;
- ffprobe post-validation required.

Engineering capability was proven in Phase 2 through `h264_mf`.

In this phase:
- integrate it in the real UI/export workflow;
- test end-to-end on Windows CI;
- clearly distinguish engineering completion from legal/product patent decisions.

Do not falsely claim legal resolution.

---

# 40. Keyboard shortcuts

Implement configurable keymap.

Provide:
- default CapIA preset;
- editable bindings;
- conflict detection;
- reset to defaults.

Initial commands include timeline/navigation/editing shortcuts from `TIMELINE_UX.md`.

---

# 41. Internationalization

UI must support:
- pt-BR;
- en.

Requirements:
- externalized strings;
- language switch;
- no major hardcoded user-facing text;
- persisted preference.

Do not translate internal IDs/errors.

---

# 42. Accessibility

Baseline:
- keyboard navigation;
- visible focus;
- accessible labels;
- contrast;
- menu/dialog navigation;
- tooltips for icon-only controls.

Do not block the entire phase on exhaustive WCAG certification.

---

# 43. Workspace persistence

Persist practical local UI preferences:
- panel widths;
- collapsed panels;
- timeline track heights;
- zoom where appropriate;
- active language;
- selected keymap;
- preview quality/proxy preference.

Project data and UI preference data must remain separated.

---

# 44. Autosave/status UX

Expose:
- saved/saving state;
- background job status;
- media processing status;
- export status;
- recoverable errors.

Do not show noisy internal logs to normal users.

---

# 45. Error UX

Map structured backend errors into useful UI messages.

Examples:
- offline media;
- conflict;
- locked track;
- invalid trim;
- unsupported encoder;
- export exists;
- project busy.

Preserve technical details in expandable diagnostics/logs.

---

# 46. Desktop/Tauri integration

Use the existing desktop shell.

Required:
- IPC contracts typed;
- no uncontrolled arbitrary command bridge;
- cancellation for long operations;
- event subscriptions cleaned up;
- no memory leak from repeated project open/close.

---

# 47. UI architecture

Prefer:
- frontend read model;
- incremental patches/events;
- commands through backend facade;
- local gesture preview;
- authoritative commit through engine.

Avoid:
- reserializing the entire project on every mouse movement;
- direct frontend mutation of authoritative document;
- per-frame IPC for timeline interaction.

---

# 48. Main command rule

**Every document mutation from the UI must become a Command Engine command or transaction.**

Add automated architecture/E2E checks where practical.

No hidden direct writes.

This is a hard Phase 3 requirement.

---

# 49. Performance instrumentation

Add lightweight dev metrics for:
- timeline render FPS;
- visible clips;
- paint time;
- gesture calculation time;
- command round trip;
- preview dropped slots;
- preview latency;
- thumbnail latency.

Do not ship heavy telemetry.

---

# 50. Large-project stress

Create a development fixture/project approximating:
- thousands of clips;
- many tracks;
- nested sequences;
- thumbnails/waveforms.

Use it to measure:
- open;
- zoom;
- scroll;
- selection;
- drag;
- preview.

No need to version huge media assets.

---

# 51. E2E Windows

Use an appropriate Windows desktop/UI automation approach.

Critical flows:

1. launch app;
2. create/open project;
3. import media;
4. drag to timeline;
5. trim;
6. split;
7. move;
8. undo/redo;
9. add text;
10. add caption;
11. add transition;
12. edit audio;
13. nested sequence navigation;
14. offline/relink flow;
15. export H.264;
16. reopen project;
17. validate persisted result.

Prioritize these E2Es over excessive low-level UI unit tests.

---

# 52. Visual regression

Use small targeted screenshot/golden tests for:
- main editor shell;
- timeline with clips;
- offline state;
- inspector;
- export dialog.

Do not create hundreds of brittle screenshot tests.

---

# 53. Crash/recovery smoke

At minimum test:
- kill app after edits;
- reopen;
- project integrity;
- no partial export published;
- no corrupt UI preference file.

Reuse backend guarantees rather than duplicating exhaustive Phase 2 crash matrices.

---

# 54. CI

Final Phase 3 CI should include:

## Windows
- Rust workspace;
- TypeScript/frontend;
- desktop build;
- UI/E2E smoke;
- timeline performance smoke;
- preview P2 functional smoke;
- H.264 export E2E;
- architecture checks.

## Linux
- backend regression;
- frontend lint/typecheck/build;
- relevant headless UI tests;
- render parity;
- licenses/security.

Do not allow the full UI suite to make CI unreasonably slow.

Separate:
- PR/smoke;
- heavy/nightly/manual performance where needed.

---

# 55. Human acceptance package

One ROADMAP criterion cannot be honestly completed autonomously:

> experienced editor creates a 30–45s UGC ad in ≤15 min, no blocking bugs, with ≥3 users.

Do not fabricate this.

Prepare:

`tools/phase3-acceptance/`

It should contain:
- sample project/media generation or safe test assets;
- exact task script;
- timer;
- checklist;
- issue severity form;
- automatic collection of non-sensitive performance logs;
- result summary template.

The human task should be easy to run.

---

# 56. Acceptance task

The manual acceptance scenario should produce:

- talking head;
- B-roll;
- text;
- manual captions;
- music;
- SFX;
- at least one transition;
- at least one trim/split;
- export MP4.

Target duration:
30–45 seconds.

Measure:
- completion time;
- blocking bugs;
- major confusion points.

Need ≥3 users for the exact ROADMAP checkbox.

---

# 57. GPU residual package

If real-GPU ADR-069 residual cannot be executed autonomously, include it in the acceptance package.

One-click command:
- runs P2 at 1280×720;
- collects CPU/pacing;
- reports threshold pass/fail.

Do not require the user to manually inspect logs.

---

# 58. Phase completion semantics

There are two legitimate terminal states:

## A. `PHASE 3 COMPLETE`

Allowed only if:
- engineering is complete;
- automated criteria pass;
- real-GPU residual passes or does not trigger reopen;
- H.264 engineering gate passes;
- ≥3-user acceptance criterion has real results.

## B. `PHASE 3 ENGINEERING COMPLETE — HUMAN ACCEPTANCE PENDING`

Use when:
- all implementation is done;
- CI is green;
- editor is usable;
- only unavoidable external human/hardware acceptance remains.

Do not falsely mark the ROADMAP human checkbox.

This state is acceptable for handing off to the user while avoiding idle engineering time.

---

# 59. Do not start Phase 4

Do not implement:
- OpenAI/Anthropic/Google provider abstraction;
- chat;
- transcription;
- AI captions;
- media analysis AI;
- Reference Analyzer;
- DemandSpec;
- AI commands;
- model registry;
- cost accounting.

Only Phase 3 manual editor work.

---

# 60. Documentation

Update:
- `docs/STATUS.md`
- `docs/ROADMAP.md`
- `docs/TIMELINE_UX.md`
- `docs/PREVIEW_RENDER.md`
- `docs/ARCHITECTURE.md`
- `docs/DATA_MODEL.md`
- `docs/COMMAND_SYSTEM.md`
- `docs/TEST_STRATEGY.md`
- `CLAUDE.md`
- UI/desktop READMEs.

Create ADRs only for genuinely architectural decisions.

Do not generate ADRs for routine component choices.

---

# 61. Autonomous execution rules

The user will not be available continuously.

Do not pause for:
- styling choices;
- component naming;
- test fixes;
- CI fixes;
- small refactors;
- library selection that respects architecture/license constraints.

If a path fails:
- diagnose;
- replace/fix it;
- continue.

If a task is blocked by human-only acceptance:
- mark it pending;
- continue all engineering tasks.

---

# 62. Commits and CI

Use meaningful vertical-slice commits.

Suggested milestones:
1. shell/design system;
2. project/media navigation;
3. timeline renderer;
4. editing gestures;
5. preview;
6. inspector/keyframes;
7. text/captions/transitions;
8. audio;
9. export/deliverables;
10. E2E/performance/accessibility/i18n;
11. acceptance package;
12. docs/final audit.

Push periodically.

Avoid cosmetic pushes while final CI is running.

---

# 63. Final audit

Before declaring engineering complete, manually inspect:

- any UI direct document mutation;
- stale read-model state;
- gesture race conditions;
- selection drift;
- timeline virtualization artifacts;
- canvas memory leaks;
- dangling WebView event subscriptions;
- preview buffer leaks;
- resize/DPI;
- offline assets;
- nested navigation;
- undo/redo after complex UI edits;
- keymap conflicts;
- untranslated UI strings;
- export cancellation;
- unsupported H.264 path;
- panel persistence corruption.

---

# 64. Definition of Done — engineering

Engineering completion requires:

- editor shell usable;
- media import usable;
- timeline usable;
- project/sequences usable;
- manual clip editing usable;
- nested usable;
- preview P2 integrated;
- inspector usable;
- keyframes usable;
- text usable;
- manual captions usable;
- basic transitions usable;
- audio editing usable;
- history usable;
- relink usable;
- export/batch deliverables usable;
- H.264 Windows path integrated;
- shortcuts configurable;
- pt-BR/en;
- critical E2E Windows green;
- performance instrumentation;
- timeline stress validation;
- no direct UI writes around Command Engine;
- docs updated;
- working tree clean;
- final CI green.

---

# 65. Final report

Return:

## Status
- PHASE 3 COMPLETE
or
- PHASE 3 ENGINEERING COMPLETE — HUMAN ACCEPTANCE PENDING

## Git
- branch
- commit
- main commits
- final CI

## UI
- shell
- design system
- project/media
- timeline
- inspector

## Editing
- move
- trim
- split
- snapping
- groups
- copy/paste
- tracks
- nested
- keyframes

## Content
- text
- captions
- transitions
- audio

## Preview
- P2 implementation
- real-GPU residual status
- measured latency/CPU/dropped slots where available

## Export
- deliverables
- batch
- H.264 path
- ffprobe validation

## Performance
- timeline
- command latency
- scrub
- preview

## Tests
- unit/integration
- E2E
- visual smoke
- crash smoke

## i18n/accessibility

## Human acceptance
- package path
- tests completed/pending
- exact remaining actions

## Blockers

## Next allowed phase
Only recommend Phase 4 after the Phase 3 completion state is truthfully documented.
