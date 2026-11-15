# Skylis parser allocation limits

Upstream: `sevenz-rust2` 0.25.0, crates.io checksum
`e5e465701779cbc6d0d10cf907f175035cfdd50b8d3cb628d7f1f435ef02972f`.
License: Apache-2.0; the upstream `LICENSE`, manifest, README and source are retained.

The upstream memory limit applies separately to encoded header buffers and individual
decoder models. Counts are checked against the header byte length; a large malicious
header can therefore declare many more file records than an application accepts and
allocate their vector before the application can inspect the entries.

This patch adds a conservative, per-thread parser allocation budget scoped to
`Archive::read_with_memory_limit`. Array capacities, bitsets, UTF-16 scratch buffers,
UTF-8 names and LZMA/LZMA2 encoded-header dictionaries are charged before allocation.
The budget uses the caller's existing memory limit (128MiB in Skylis), and counts
cumulative allocation requests, even when temporary allocations have been freed.
The guard restores prior thread-local state on normal return, errors and unwinding.
Normal data decoding happens outside the parser guard; Skylis additionally bounds
the combined decoder dictionaries of each content block before extraction.

Declared file, block, packed-stream and sub-stream counts are capped at 100,000
before vectors are reserved. Names are capped at 4096 UTF-8 bytes per entry and
32MiB cumulatively before allocating their output strings. Header byte ranges still
use upstream bounds rather than the entry-count bound.

These changes do not claim a process RSS ceiling: allocator overhead, input handles,
the UI and cached indices are separate. They bound untrusted parser allocations and
avoid interpreting the original per-dictionary limit as a global memory guarantee.

The archive adapter regression tests cover a declared count above the limit, metadata
budget failure, ordinary ZIP/7z roundtrips, solid selected extraction and corruption.
The UI runs at most one inspection and one content operation concurrently.

Remove or replace this patch when upstream provides equivalent pre-allocation limits.
