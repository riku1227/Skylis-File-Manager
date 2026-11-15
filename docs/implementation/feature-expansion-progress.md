# Feature expansion progress

Plan/spec: feature-expansion-design.md. User explicitly delegates specification and execution; no intermediate design approval required.

- [ ] Icons: scheduler/cache regression tests, bounded demand implementation, cargo test, feature commit.
- [ ] Settings: versioned persisted preferences and live settings UI, round-trip/corruption tests, feature commit.
- [ ] Downloads: pure date grouping and virtual row mapping, boundary tests, feature commit.
- [ ] Scrollbars: virtualized scroll geometry and draggable interaction, geometry tests, feature commit.
- [ ] Context menu: custom popup + native Shell host, cancellation/selection tests and native smoke check, feature commit.
- [ ] File drag/drop: native/internal transfer with safe collision policy and background jobs, filesystem tests, feature commit.
- [ ] Tab drag/drop: reorder/detach/merge with stable folder ownership, tests and manual check, feature commit.
- [ ] Preview: Space/Escape lifecycle, bounded image/text and platform fallback, tests, feature commit.
- [ ] WSL: asynchronous distribution discovery and UNC locations, parsing tests, feature commit.
- [ ] Gallery: 80% classification, bounded progressive thumbnails, cancellation/cache tests, feature commit.
- [ ] Folder sizes: bounded recursive scanner + per-view queue, cancel on navigation, tests, feature commit.
- [ ] Final review, tests/build, standalone temporary graphical HTML guide, remaining decisions.

Baseline: main had only README.md modified and docs/implementation/windows-build.md untracked. Feature branch created for this work. Opus 5.5 design consultation launched. Implementation workers have disjoint file ownership; integration and commits are serialized by root.
