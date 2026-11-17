# Recursive name search under the current folder

Date: 2026-11-17. Status: implemented; Opus completion findings corrected and rechecked.
Author: Astra. Numeric limits and interaction restrictions below are proposed engineering
choices, not decisions attributed to the user. The requested workflow is design → Opus
design review → parent implementation → Opus completion review.

## Intent and scope

Find names in the current filesystem folder and its descendants without making typing,
scrolling, or navigation wait for traversal. Show direct children immediately when cheap,
then progressively show deeper matches in a separate section. Obsolete queries must lose
the right to publish immediately, even when an operating-system call cannot be interrupted.

Names use the existing lowercase substring semantics: no regex, glob, fuzzy matching,
content search, token splitting, or new Unicode normalization. Spaces remain literal;
empty text returns to ordinary browsing. Case folding remains Rust `to_lowercase`, not
locale-specific matching. Archives and Home retain their current behavior; recursion is
only for `Location::Folder`. Search does not extract archives or follow shortcuts.

Direct matches preserve the existing filter-and-act workflow, including mutations and
multi-selection. Only descendant matches are read-only in this first release.
No persistent index, recursive watcher, session schema change, or filesystem-wide search.

## Existing architecture and alternatives

`folder.rs` observes every `TextInput` notification, compares `content`, then synchronously
calls `rebuild_visible`. That lowercases every name, filters all entries, applies grouping,
rebuilds row maps, and resets scrolling. Merely adding a background recursive scan leaves
this large-directory typing problem intact. `Entry` already stores a private `name_key`.

`listing.rs` streams direct entries from a native thread; `folder.rs` merges sorted batches.
`grouping.rs` maps logical items to fixed-height `DisplayRow`s; `uniform_list` virtualizes
rendering. Selection and many commands assume indexes into `entries`/`visible`.
`transfers.rs`, context-menu fallback targets, new-item commands, and refresh/selection
restoration have current-folder assumptions. `name_operations.rs` itself correctly derives
a rename destination from the source parent; that alone does not make the surrounding UI safe.
`folder_size.rs` demonstrates cancellable DFS, reparse exclusion, and bounded handles, but
its uninterruptible I/O caveat must carry over, not its size-specific result semantics.

Considered approaches:

1. OS indexes: fast when available, but Windows/macOS scope, freshness, and provider
   differences make them unsuitable as the only implementation of this folder promise.
2. Explicit-start traversal everywhere: less background I/O, but adds a step after each
   query and does not match the requested progressive typing interaction as well.
3. **Proposed:** cached direct matching plus debounced, bounded traversal; automatic for
   verified local roots, explicit start for remote/unknown roots. Keep a separate search
  descendant projection so direct-entry indexing and mutation assumptions remain intact.

## Visible behavior

- Direct filtering follows the displayed text immediately, including IME marked text,
  without debounce. Only committed text starts the recursive debounce.
- After 250 ms without a committed query change, request descent on eligible local roots.
  No minimum character count; short queries are protected by the same hard limits.
- Use one virtualized list with `このフォルダー · N` and `サブフォルダー内 · N` headers.
  Headers remain visible with zero results; the descendant header carries its status.
  Direct children appear only in the first section, depth two and deeper only in the second.
- Use existing 24/28 px row heights. Each result shows name plus a separate, ellipsized
  relative-parent column (`.` for direct children). Full path is available through Copy
  Path and tooltip. Never use a basename as identity or reconstruct an action path from it.
- Direct order follows the existing sorted `entries` sequence; search temporarily omits
  Downloads date groups. Descendants append in discovery order, labelled `見つかった順`.
  No full-result resort on every batch or on completion; traversal order is not promised
  to be reproducible. Sorting controls affect the direct section only, with that scope labelled.
- Search temporarily uses list mode; descendants use generic icons and a text-only inspector.
  Restore the saved browsing view preference when clearing search; do not persist overrides.
- Keep accepted rows when stopped, limited, or partially failed. Clear old-query rows on
  query change; they must never look like matches for new text. Do not reset scroll on batches.
- Status distinguishes Debouncing, Classifying, RequiresStart, Queued, Running, Complete,
  Partial, Stopped, Failed, and Paused. `一致する項目はありません` is only conclusive after
  direct listing/filtering and eligible traversal both finish without incomplete coverage.
  These are display labels, not separate task machines: store phase plus optional wait/end reason.
- Stop cancels queued/running descent and retains accepted results. Retry starts a fresh
  generation with the same bounds, clears descendants, and does not secretly raise limits.
  Partial status names the reached limit or skipped categories and suggests narrowing text
  or searching a deeper folder. Search is a snapshot, not a live recursive view.
- For remote/unknown roots show `直下のみ表示中 — サブフォルダーも検索` with a Start button.
  That authorization is per query: changing its committed text requires Start again.
- Show `サブフォルダー内の結果は、開く・親フォルダー表示・パスのコピーに対応` near that section.

## Input, generations, and direct projection

Observe the pair `(content, is_composing())`, not just `content`: `unmark_text` can commit
unchanged characters. On entry to composition, cancel recursion/debounce, clear descendants,
and show `変換中 — 直下のみ絞り込み`. Direct filtering continues to follow visible marked text,
preserving current behavior; marked text never launches recursive I/O. On transition out,
debounce committed text even if its bytes equal the last marked text. Cursor/selection-only
notifications do not restart work. Cancelling composition updates direct matches to restored
text and starts a fresh debounce. Enter/Escape while composing belong to IME, not result actions.

Represent authority as `SearchKey { owner_id, navigation_epoch, query_generation }` plus
the exact absolute root `PathBuf` and hidden policy. Increment generation on text/hidden
change, retry, refresh, stop, and pause; increment navigation epoch even on same-path reload.
Every timer, batch, terminal event, and direct-filter continuation validates this key.
Cancel tokens stop work; keys reject races. Never canonicalize or stat a root on the UI thread.

Add `Entry::matches_name_key(&str)` as `name_key.contains(normalized_query)`; normalize once.
Direct filtering remains uncapped and synchronous initially. When the normalized query
extends the previous normalized query, filter the previous `visible` indexes in place;
otherwise scan `entries`. Allow narrowing ONLY in the query-change observer. Every other
`rebuild_visible` caller (listing, watch, sort, hidden, archive) does a full rebuild, preserving
existing index invariants without a new generic cache/revision mechanism. A normalized
byte-prefix check is required; do not infer it from cursor position or raw input.
Do not sort by date while searching; preserve the preference and restore it on clear.
Use arithmetic search row offsets, avoiding rebuilding date/gallery maps on each key.
Recover direct selection by path during the same pass, retaining only visible selected paths.

Initial listing merges, watch replacement, sort and hidden changes always fully recompute
direct `visible` against replacement `entries` in the SAME UI update before
render/action; never leave old indexes pointing into reordered storage. Store direct action
targets as paths when menus/async work open, and revalidate at dispatch. Descendants remain
separate. Existing listing merge/sort costs remain a known residual, not solved here.

Measure a release-mode 100,000-entry diagnostic before adding incremental machinery.
If cached matching plus selection/row projection repeatedly exceeds approximately 4 ms on
the test machine, implement slices (256 candidates, shared 2 ms quota) rather than a cap.
In that fallback show filtering progress; revision changes invalidate partial indexes before
render/actions and restart. Continue until all direct entries are processed, without a time,
entry, or result cutoff. This threshold is an engineering diagnostic, not a CI timing test.
An isolated `contains` benchmark cannot by itself establish the full UI projection cost.

Recursive query payload is limited to 4 KiB after normalization; longer text still filters
direct entries normally but shows a descendant-only query-length explanation. Empty text
restores ordinary browsing. Never interpret a descendant ordinal as `visible` or `entries`.

## Traversal policy and explicit limits

These defaults are hard ceilings checked before retaining/opening the next resource:

| Resource | Ceiling / policy |
| --- | --- |
| Process search workers | 2 long-lived threads: 1 local lane, 1 remote/unknown lane |
| Pending requests | 8 process-wide, at most 1 replaceable request per view |
| Active requests | At most 1 per view; cancelled blocked work still occupies its worker |
| DFS depth / live iterators | 64 directories including root; no breadth queue |
| Stack/frontier path storage | 512 KiB total, including retained ancestor paths |
| Individual retained path | 32 KiB native encoding (Windows UTF-16 bytes, Unix bytes) |
| Visited directory entries | 100,000, including nonmatches, skipped entries, and depth one |
| Elapsed traversal | One shared deadline, 30 seconds after submit, including queue/send waits |
| Accepted descendant matches | 5,000 per query |
| Descendant payload retained | 8 MiB for paths, names, lowercase keys and relative-parent text |
| In-flight batches | 2 queued + 1 producer batch + 1 UI-held batch per active request |
| Batch size | At most 64 matches and 256 KiB payload, whichever comes first |
| Batch cadence | Flush after 50 ms when nonempty, or at a size ceiling, or terminal |
| UI descendant work | 64 imported hits per view/drain, process-wide 2 ms per 16 ms |
| UI delivery cadence | At most one drain per view per 16 ms; notify once per changed drain |

All counters use checked/saturating arithmetic. Fixed structs/container overhead and
deduplication indexes are separately bounded by their item ceilings; the 8 MiB value is
payload accounting, not a promise about allocator RSS. Share accepted hit records with
`Arc`, including dedup keys, to avoid another full path copy. Drop the bounded old descendant
collection normally; measure this cost in repeated-query diagnostics. Do not add a retired
collection state machine unless evidence shows cleanup exceeding the update budget.

DFS retains ancestor iterators, identities, and paths only. Read root entries to discover
subdirectories, but never emit depth-one matches; normal listing is their sole source.
Count all encountered entries before filtering. Check token/deadline before and after each
`read_dir`, iterator advance, metadata call, and send. Depth/path-budget exclusions skip
that subtree and increment a partial counter; global entry/time/result/payload ceilings
end traversal with a precise Limit reason. Stop exactly at the result cap and conservatively
label limited, even if the unexamined remainder might contain no additional matches.

Reuse the no-follow intent of `folder_size.rs`, not its blanket reparse rejection. Windows
policy reads enumeration reparse tags (FindFirstFile/FindNextFile data) or obtains
`FILE_ATTRIBUTE_TAG_INFO` through a metadata-only no-follow handle for a root. Reject
name-surrogate tags (symlinks/junctions/mount points), offline, RECALL_ON_OPEN and
RECALL_ON_DATA_ACCESS directories; unknown reparse tags are skipped with their own counter.
Allow recognized CLOUD/CLOUD_n directory tags when already local and free of recall/offline
flags. The adapter, not `EntryKind::Symlink` alone, decides whether such a directory is
traversable. Apply the same policy to the root; local cloud roots are not blanket denied.
If tag retrieval fails, report policy-unknown/partial instead of guessing safe. No file data
is opened. Matching skipped directory/link names may appear, but are never descended into.
On macOS use no-follow metadata and refuse changed device IDs; keep ancestor `(device,
inode)` identities (at most 64) to reject cycles. No full-tree visited map is needed.
DFS does not follow links and retains no extra deduplication map. Different lexical
paths (including hard-link names) remain different results. No case-folding of path identity.

When hidden items are off, omit hidden matches and prune hidden directories. Honor Windows
hidden/system attributes and Unix dot names using the same policy as `Entry`; switching
visibility cancels and restarts search. An explicitly revealed hidden direct item may keep
the existing launch exception; that exception must not grant recursive hidden traversal.

Root classification and traversal safety live behind a small `SearchFilesystemPolicy`
adapter, called only on the fixed search workers. Windows uses path syntax plus drive-type
classification: UNC, `\\wsl$`, `\\wsl.localhost`, mapped remote drives, removable, and
unknown volumes require explicit Start. macOS uses filesystem locality flags; remote or
unknown mounts require Start. Other targets default to RequiresStart until classified.
Verified local fixed roots may start automatically. A name-surrogate/recall/offline root
remains direct-only with an explanation; an eligible already-local cloud root may recurse.
Cross-volume descent is always excluded;
explicit Start authorizes the selected remote root, not links/mounts encountered below it.

No file content, thumbnails, shell properties, or recursive sizes are read. Do not explicitly
hydrate placeholders; apply the tag/recall policy above to Windows directories. Metadata/listing
can still cause a cloud provider or network filesystem to perform I/O, so do not claim
zero network access or guaranteed no hydration on every provider. This limitation is
documented; native policy cannot fully identify all provider-backed ordinary directories.

## Scheduler, backpressure, and late I/O

Use a process-wide `SearchService`: two fixed lane workers, a small mutex-protected queue
and condition variable. The local lane accepts only verified local roots; all unknown/root
classification work and explicit remote traversal use the other lane. Classifying a new
root may queue behind a blocked remote job, but already verified local roots still progress.
Only a non-I/O cached/path classification can route immediately; do not probe drive metadata
on UI. A successful local classification transfers the same request/deadline to the local
lane, without occupying two pending slots. Cache classification for the navigation epoch.
This isolation prevents a known remote scan from consuming the local traversal worker;
it cannot promise that an apparently local filesystem never blocks.

UI submission only replaces/removes a small pending record. Queue overflow returns Busy
with Retry, not repeated automatic timers. FIFO per lane, with latest-per-owner replacement,
avoids starvation among eligible owners. An owner with cancelled native work still running
cannot concurrently launch replacement work on another lane. Thread failures report Unavailable.

Use ONE bounded FIFO `async_channel<SearchEvent>` of capacity two per request, with
`Batch | Progress | Done`. Every envelope carries SearchKey/root. Progress is at most once
per 100 ms and may be dropped via try_send when full; never discard a match batch or reorder
Done ahead of it. Flush the last batch before Done. Cancellation closes the receiver and
wakes a blocked sender. Sending checks the common deadline via timed wait/try-send loop;
UI watchdog uses that same deadline, closes the receiver and reports timeout if necessary.
No terminal/status side-channel is needed; timeout and explicit cancellation are UI-owned.

Each view drains at most 64 matches, retaining at most one partial batch, then yields for
16 ms. A tiny shared `SearchDrainBudget` grants at most 2 ms of aggregate import/selection
work per 16 ms across all windows, checked between bounded 64-hit imports (a chunk can
overshoot the cooperative budget). Maintain a round-robin ready-owner deque with deduplicated
IDs: at most eight subscribed result streams globally, including finished workers whose
queues are not drained. Admission/closing releases a subscription independently of worker
occupancy. Overflow owners show Busy/Retry rather than building an unbounded ready queue.
Each view has at most one scheduled continuation; a view yielding the quota moves to the
back, so the first window cannot consume every allowance. Notify once per changed drain.
The quota is cooperative, not realtime: charge elapsed time after each bounded import. Direct
synchronous filtering and existing listing/paint costs are measured separately; if the
conditional direct-slicing fallback is needed, its slices consume this same global quota.

An OS call already blocked inside `read_dir`/metadata cannot be cancelled by a Rust token.
Do not spawn replacement threads, detach one thread per keystroke, or join on UI/quit.
Workers keep occupying the two fixed slots until calls return. A new query can show
`前の読み取りの終了を待っています` while direct filtering remains usable. A UI watchdog
marks a request timed out at its shared submit-time deadline and invalidates its authority;
it cannot reclaim the native worker. When the call returns, drop its results and take the
latest eligible queued request. Worker teardown must not prevent application exit.

## Projection, selection, and action safety

Keep ordinary `entries` and `visible` semantics intact. Add `DescendantSearchState` and a
tagged row reference `Direct(visible_ix) | Descendant(hit_ix) | Header(section)`;
never insert descendant entries into `entries` or store their ordinal in `self.selected`.
Compute header/direct/descendant row offsets arithmetically rather than rebuilding a large
combined row vector per batch. A new direct match shifts the descendant section; preserve
the first visible result's path and pixel offset so progressive updates do not jump it.
Headers are never selectable. `uniform_list` renders only requested rows.

Direct results retain existing single/multi-selection, range/marquee selection, Ctrl+A and
file drag semantics. Descendants support single selection only; store selected hit index
plus captured path/SearchKey, validating all three before actions. Append-only results
need no second path-to-index map. Crossing sections with click/arrows clears the other
section's selection; headers are skipped by arrows/Home/End/PageUp/PageDown. Shift-range
and marquee begun in direct rows clamp to the direct section. Ctrl+A selects direct rows
only when direct scope is active; in descendant scope it does nothing with visible help.
Never form a mixed-parent transfer selection. Escape first respects IME, then clears search.
Text-input Copy/Cut/Paste continues editing text; result Copy Path is a distinct command.

Allowed descendant commands: Open, Open Folder in New Tab when applicable, Reveal Parent in
Skylis, Copy Path, Stop, Retry/Clear. Extract filesystem opening into an explicit
`open_filesystem_target(path, kind, authority)` helper shared with ordinary `open_entry`.
Preserve archive-file opening and existing asynchronous file/link-opening behavior.
Reveal Parent navigates to `hit.path.parent()` with the actual selected filename/path;
never join `current_root` with `hit.name`. Extend pending selection to a path/OsString if
needed so lossy display names cannot select a different non-Unicode filesystem entry.
Revalidate query authority at dispatch and async completion; disappeared paths report an
ordinary open error. SearchKey is UI authority, not a security guarantee against filesystem races.

Guard commands by explicit action target/selection scope, NOT `query != empty`. Direct
Rename/Delete/Copy/Cut/Paste, New Item, archive operations, Properties, native menu and
preview keep existing behavior. Descendant scope rejects those commands at handler entry
and menu construction, including toolbar/keyboard routes. Context menus capture a tagged
path target; stale descendant menus validate generation, and direct menus revalidate path
against current entries. Descendant menus/blank descendant
space never fall back to `self.path()`. To edit a descendant, Reveal Parent returns to browsing.
Background commands remain available on explicitly direct-section background. With descendant
selection, keyboard Paste/New Item must reject rather than quietly modify current root.

Direct row/folder drops and direct-background drops retain existing validated destinations;
descendant rows, headers and descendant background are not drop targets or drag sources.
Route hit testing explicitly, including outer `selection_surface`/background drop receivers,
so an event rejected by a descendant cannot bubble into a root drop. Direct selection clears
descendant scope before invoking existing actions, and vice versa clears `multi_selection`,
anchor and `selected`. Audit every action route, not only the rendered menu.

Descendants trigger no icon extraction, decoding, gallery, preview or size jobs. Direct
rows retain existing visible-only icons/preview and bounded media scopes. Gallery is only
a temporary presentation override; valid existing cached/displayed images are retained.
MediaCache bounds (64 MiB, 384 entries, 96 queued jobs) are not enlarged; traversal never
requests media for any hit. Descendant text inspection does not reach `selected_entry`.

## Lifecycle and module boundaries

- `crates/explorer-core/src/name_search.rs` (new): types, limits, name matching/traversal,
  progress/terminal reasons; injectable filesystem/clock seam for deterministic tests.
- `crates/explorer-core/src/name_search/policy.rs` (new): narrow Windows/macOS/no-follow
  classification adapter; export through `lib.rs`. Add cached-match accessor in `entry.rs`.
- `apps/explorer/src/name_search.rs` (new): GPUI-independent fixed worker scheduler,
  bounded delivery and cancellation ownership; initialize one shared service in app setup.
- `apps/explorer/src/folder/name_search.rs` (new): descendant state, debounce/IME logic,
  drain/watchdog/quota, tagged row routing, descendant selection and restricted actions.
- `folder.rs`: route search observer, list/keyboard/status rendering, full-rebuild invalidation,
  navigation/drop cancellation, and shared path-based opening into these narrow helpers.
- `grouping.rs`, `media_view.rs`, `transfers.rs`, `context_menu.rs`, name-edit/delete/archive
  action entry points: descendant-scope guards; keep direct command behavior and indexes.
- `folder/watch.rs`: snapshot changes invalidate direct projection; mark descendants
  `更新された可能性があります — 再検索` without launching recursive rescans on each event.
- `explorer.rs`/`explorer/tabs.rs`: explicitly tell old/new tabs when selected status changes,
  including detach/reparent/close. Do not infer activity solely from `FolderView::render`.

Navigation, clear, drop, refresh, and tab deactivation close receivers and cancel timers and
tokens. Inactive tabs retain accepted result rows but no pending traversal; returning shows
Paused with Retry, avoiding automatic I/O on repeated tab switching. Reparenting preserves
entity identity but refreshes owner window routing and invalidates old completion handlers.
Restored inactive tabs launch no search; query/results are not persisted by `session.rs`.
Explicit Refresh refreshes direct listing and starts a new search generation once the root
is ready. Ordinary root watch refreshes do not claim descendant freshness. Mutation events
from other tabs may mark the search stale when their affected directories lie below its root.

Proposed interfaces: `SearchRequest { key, root, query_key, show_hidden, explicit_start,
cancel, deadline, limits }`; `SearchHit { path, name, kind, relative_parent }`;
`SearchEvent { key, root, payload: Batch | Progress | Done }`; `submit -> Accepted | Busy |
Unavailable`; `cancel_owner(owner_id)`. Each view owns one live receiver/timer set.

## Acceptance and evidence

Deterministic core/scheduler tests must cover:

- Direct/deep separation without duplicate root children; substring/case/Unicode behavior;
  same basenames in multiple parents; hidden subtree pruning and hidden-policy restart.
- Symlink cycles, Windows name-surrogate exclusion, root link refusal, mount boundaries,
  mapped remote/UNC/WSL classification and unknown policy. Test local CLOUD/CLOUD_n root
  and child acceptance separately from offline/recall/unknown-tag skips, with reported counts.
- Every numeric ceiling at boundary and one beyond; wide and deep trees; inaccessible root
  versus partial child failures; removed entries; non-Unicode paths; hard-link names.
- Block a fake iterator call, replace query repeatedly, and assert no more than two workers,
  eight pending requests, one pending per owner and eight undrained stream subscriptions.
  Block remote lane and prove verified local jobs progress; release late I/O and reject output.
- Full result queue plus cancellation/deadline; closed receiver; terminal ordering; no lost
  accepted last batch; worker-start failure and Busy retry; bounded retained path accounting.

GPUI/model tests must cover IME mark/replace/unmark with unchanged commit bytes, composition
cancel, direct marked-text filtering, cursor-only updates, empty-query restoration,
prefix narrowing versus full rescans, late listing/watch merges, stale direct indexes,
and aggregate drain quota/fairness with more completed queues than active workers.
Verify headers are skipped by keyboard movement, path selection survives append, scrolling
is anchored as direct rows arrive, and stale query/root/tab events cannot change the view.
Prove direct rename/delete/copy/cut/paste/multi-select/range/drag/preview still work under a
query. Dispatch prohibited descendant commands through shortcut/menu/toolbar; assert no
root fallback, no mixed-section selection, and no descendant drop bubbling to root receiver.
Open/reveal two identical names in distinct descendant parents; verify the exact target.
Check search in Downloads/gallery/archives/Home and deferred restored tabs for regressions.

Run formatting and relevant core/app tests, then workspace checks supported by this machine.
Use a repeatable manual diagnostic fixture (e.g. 50,000 direct entries plus 100,000 nested
entries) to record first-direct/first-descendant latency, UI slice durations, cancellation
publication latency, max live workers/queued batches/path bytes, and memory before/after
repeated queries. Report hardware, build mode, tree shape, cache state, and actual values.
Do not add flaky wall-clock pass/fail assertions to CI; fake clocks prove scheduling logic.

Real Windows/macOS runs must inspect typing, IME, scroll/selection and frame-time traces;
compare p50/p95/p99 frame times and worst search-update slices with baseline browsing.
Unit tests and instrumentation do not prove zero frame drops. Native Windows GUI launch was
previously blocked by antivirus here, and macOS/real frame-time checks are unavailable in
this environment. Record those as unverified, not passed; keep evidence separate from goals.
Remaining risks are blocked native I/O, pre-existing direct listing merge costs, provider
behavior, and the deliberately restricted descendant action set.

## Review revision and implementation sequence

Opus blockers accepted: preserve direct filter-and-act commands; remove direct result/time/
candidate caps; use cached-name synchronous filtering first; make cloud policy tag-aware.
Simplifications accepted: local/remote worker lanes, one FIFO event channel, one deadline,
no duplicate descendant path-index map or speculative retired-result state machine.
Retained deliberately: path/frontier/8 MiB payload limits (5,000 paths are not necessarily
small), deterministic blocked-I/O seams, and a lightweight global drain quota. Two active
workers alone do not bound simultaneous imports from already-completed requests.

Parent's scratch diagnostic: Rust 1.100.0 `-O`, Ryzen 7 5700X, Windows, 100,000 names,
10% Japanese+Project / 90% Project_report_and_documents, 41 warm in-memory iterations.
It measured index filtering plus a simple row-tuple vector, NOT actual GPUI/Entry projection:

| Query / hits | Per-row lowercase p50/p95/max ms | Cached p50/p95/max ms |
| --- | --- | --- |
| project / 100,000 | 9.463 / 10.553 / 14.626 | 1.502 / 1.766 / 2.103 |
| project_report_and_documents_09 / 9,000 | 9.710 / 10.641 / 10.917 | 2.967 / 3.481 / 4.012 |
| Japanese / 10,000 | 8.276 / 11.954 / 13.187 | 0.748 / 1.307 / 1.423 |
| no match / 0 | 6.648 / 9.550 / 13.450 | 0.706 / 0.976 / 1.537 |

This supports cached synchronous filtering first on this host, not a frame-time guarantee.
After phase one, measure actual `Entry` filtering/selection/row mapping; add uncapped slicing
only if the approximately 4 ms diagnostic criterion warrants it. Keep scratch probes uncommitted.

Implement in four independently reviewable Conventional Commits with bodies and validation:
1. `perf:` cached direct filtering, observer-only narrowing, search grouping suppression;
   direct-action regression tests and actual release-mode projection diagnostic.
2. `feat:` bounded core traversal and tag-aware policy, deterministic limits/I/O tests.
3. `feat:` fixed service/local auto-search UI, tagged action routing, shared drain quota,
   lifecycle/IME/selection integration. Remote remains honestly direct-only at this stage.
4. `feat:` explicit remote/unknown Start, remote lane routing/status and isolation tests.
Then Opus reviews implementation against this revised design and measured evidence.

## Implementation evidence and adjustments (parent, 2026-11-17)

The final implementation uses a metadata-only Windows handle to resolve intermediate
junctions before drive classification. The fixed worker loop catches job panics and
continues serving after reporting a failure. Full result channels use a deadline-aware
5ms retry on the worker; UI cancellation also closes the ticket. macOS dataless
directories are excluded using SF_DATALESS; native macOS behavior is unverified.

The direct projection diagnostic over the actual GPUI folder model (100k Entry values,
41 iterations per query, release on Ryzen 7 5700X) after precompiling the substring finder:
project p50/p95/max 2.380/2.826/3.348ms; long substring 2.981/3.570/3.720ms;
Japanese 1.752/2.126/2.183ms; no match 1.907/2.554/3.151ms. No slicing was needed
under the measurement gate. These timings exclude native painting and GPU frame time.

Descendant actions are available in a text inspector and a compact selected-result bar,
including with the inspector off. Right-click selects a descendant but opens no native
mutation menu. F5 starts a fresh bounded descendant request while the existing direct
watch refresh completes; normal watch updates preserve the descendant snapshot without
starting another traversal. Scroll anchoring follows the first visible path or stable
descendant index during direct listing/watch revisions.

Completion review corrections: Darwin metadata trait and statfs flag types were fixed
and the core passed aarch64-apple-darwin cargo check. Same-active switches preserve
search; background-tab drag activation pauses the previous entity before reorder;
background detach preserves the visible source tab. Settings use explicit visibility
events. Descendant open authority is cancelled on pause, tab close and entity drop.
Windows metadata paths use normalized absolute UTF-16 verbatim paths, preserving UNC
and non-Unicode names; a real >260-character fixture passed. Opus's final focused
recheck found no remaining Important issues. Native caveats remain as documented.
