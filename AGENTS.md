# Skylis development conventions

- Preserve the application's lightweight character: no filesystem scans, shell extension calls, image decoding or recursive size calculations on the UI thread.
- Bound worker concurrency, pending work and image memory. Cancel obsolete requests on navigation; never replace a valid displayed image with a loading placeholder.
- Keep platform integrations behind small, explicit adapters. Report unsupported capabilities honestly.
- Use Conventional Commits (`feat:`, `fix:`, `perf:`, `docs:`, etc.). Commit each independently usable feature separately.
- After the subject and a blank line, write a body explaining the intent, user-visible behavior, important constraints and validation. Do not use subject-only commits for feature work.
- Stage explicit files/hunks; preserve unrelated local changes. Do not commit temporary explanation HTML or local verification artifacts.
