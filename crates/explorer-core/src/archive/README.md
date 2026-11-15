# Archive worker backend

`archive::inspect`, `extract`, `create` and `source_is_current` are blocking worker
APIs. Only `detect_format` and `children` are suitable for UI use. An index stores
normalized relative member paths; no archive member is a physical writable path.

Supported input is single-volume ZIP (Stored, Deflate, Deflate64, ZIP64) and 7z
(Copy, LZMA, LZMA2, BCJ/BCJ2 and Delta filters). Output is ZIP Deflate level 6 or
7z LZMA2 level 3, with one compression thread and independent member blocks.
Selected extraction also supports existing solid 7z archives; skipped members are
drained with cancellation, output limits and integrity checks.

Passwords, split volumes, self-extracting ZIP offsets, unsupported codecs and
archive editing are reported as unsupported. No password-protected archive is
treated as empty. ZIP names use a validated Unicode Path extra field/UTF-8 first,
then strict CP932 decoding for Japanese legacy ZIPs, then the library's CP437
fallback. ZIP creation preserves representable DOS timestamps; 7z preserves NT time.

## Publication and platform behavior

Extraction always publishes a new uniquely named folder, preserving selected
members' archive-relative parent paths. Compression requires a destination that
does not exist. Both stage beside the final destination and publish through a
no-replacement rename only after successful completion. Failure/cancellation
cleans only the operation's owned stage. Metadata fingerprints and fresh member
tables detect changed source archives before extraction; decoded data is checked
for CRC/corruption and declared-size mismatches.

Traversal, absolute paths, reserved/ambiguous Windows names, duplicate and case
collisions, archive links/reparse members, filesystem name-surrogate reparse points,
and 7z anti-items are rejected. Existing extraction directories are accepted only
when that exact relative spelling was created during the same operation; NTFS 8.3
aliases cannot silently merge two archive directories.

Windows cloud reparse tags are distinguished from links through
`FileAttributeTagInfo`. Cloud files use normal reads for hydration, with the opened
handle checked again. Worker-only index-cache checks share this policy. Actual
OneDrive hydration was not tested against a user's live account; tag classification
and ordinary file behavior have regression coverage.

Windows source volume `FILE_NAMED_STREAMS` capability is queried before reading
`Zone.Identifier`. A volume without ADS has no mark; an unknown capability/read
failure aborts. An existing mark is bounded to 64KiB, copied and verified on every
extracted regular file before publication. A marked archive cannot be extracted to
a destination that cannot retain its mark. Raw Win32 rename/free-space calls use
extended-length drive/UNC paths. A real create/extract test exceeds MAX_PATH.

Stage directories use mode 0700 on Unix; Windows stages inherit the destination
ACL. Reparse checks are not an isolation boundary against another hostile process
running as the same user and replacing filesystem objects between checks.

## Resource policy

| Resource | Bound |
| --- | --- |
| Archive members | 100,000 |
| Member-name bytes | 32MiB total; 4096 bytes per path |
| Expanded member | 16GiB |
| Expanded archive | 64GiB, including skipped solid members |
| Copy buffer | 256KiB |
| ZIP central directory | 128MiB, preflight before library allocation |
| 7z parser allocation budget | 128MiB cumulative charged allocations |
| 7z content decoder dictionaries | 128MiB combined conservative estimate per block |
| Content decoder threads | 1 |
| UI content operations | 1 running, 4 pending |
| UI inspections | 1 running, 8 pending |
| UI history | 16 records |
| UI progress | at most 10 updates/second; coalesced wake queue of 1 |

7z parsing uses a documented local pre-allocation patch in
[`vendor/sevenz-rust2/SKYLIS-PATCH.md`](../../../../vendor/sevenz-rust2/SKYLIS-PATCH.md).
These limits are not a total process RSS guarantee: allocator overhead, the UI,
cached indices and concurrent inspection/content workers are separate.

## Dependencies and validation

Production dependencies: `zip = 9.6.0` (MIT), `sevenz-rust2 = 0.25.0`
(Apache-2.0), its `lzma-rust2 = 0.25.0` codec (Apache-2.0), and `encoding_rs`
0.8.44 (`(Apache-2.0 OR MIT) AND BSD-3-Clause`). ZIP default features are disabled;
only Deflate using the Rust zlib backend and Deflate64 are enabled. Sevenz default
features are disabled; only compression is enabled. Vendored upstream license
and source are retained. Release executable size is measured by the application
build, not inferred from dependency count.

Core regressions cover ZIP/7z Unicode/binary/empty-directory roundtrips, CP932 and
Unicode Path extras, selected solid extraction, no overwrite, cancelled/failed
stage cleanup, corruption, size/count budgets, an adversarial padded 7z header,
parser budget reset after failure, encrypted headers, MOTW, path aliases and long
Windows paths. Service tests cover queue/history bounds, queued/running cancellation,
worker recovery/shutdown and off-thread cache reuse/invalidation.
