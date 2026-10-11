# Shared state DB contention (#359, #324, #299 follow-up), 2026-10-11

Seven or eight `corbanu exec` sessions on one home (578 MB `pfterminal_state_5.sqlite`,
61,042 ledger attempts) logged `developer accounting failed step="open sampling" ... after 2
contended retries over 15874 ms: ... database is locked`, the "shared state database is busy"
note, failed thread-metadata writes and, in the TUI, "failed to check provider request throttle
state". Every model request could lose 15-30 s.

## What held the lock

Measured with `codex_state::db_lock` (new: every `BEGIN IMMEDIATE` logs its wait and hold with
its call site; a hold over 250 ms is a warning) on a `.backup` copy of the live DB, never the live
file.

| Holder | Before | Why |
| --- | --- | --- |
| Every ledger write (`accounting_store.rs` `write`, `open`) | 0.3-0.9 s each, up to 11 s, 4-6 per request | `check_day` and the retention check found the owner's day and the due expiries with `json_extract` over all 61k attempt payloads, under the lock. A debug build also ran SQLite's C code unoptimized (3-4x slower). |
| Each process's first write of every UTC hour | 75 s of CPU before the request (debug) | Whole-ledger validation; every process did it. |
| Conversation delete | full retention sweep per owner, twice with the #308 preflight | #324 |
| Thread metadata, throttle | waited SQLite's 5 s busy timeout (throttle: up to 10 s) behind the above | #359, #351 |

## Fix (see the PR)

- Ledger lookups use two expression indexes on `draft_accounting_attempts` (owner+dispatch,
  dispatch), created idempotently by the writer, not by a ledger migration, so older builds keep
  reading and writing the ledger. On the 578 MB copy: 154 ms under the lock after a lock-free
  warm-up read; the DB grew 4 MB. An older binary (`corbanu-before`) then ran a turn and recorded
  normally on that DB.
- `libsqlite3-sys` at opt-level 3 and `codex-state`, `sqlx-*`, `serde_json` at opt-level 2 in dev
  builds (developer accounting is debug-only): validation 75 s -> 29 s.
- Per-process ledger writer: a request hands its record over (saved first under
  `accounting-pending/`), waits only while the write meets no lock (at most `QUICK_BUSY_TIMEOUT`
  = 500 ms, or 1 s total), then goes on; records are written later, never dropped for contention.
  Records of exited processes are adopted by the next writer. One process at a time runs the
  hour's validation (file lease); `exec` waits at exit only while its own writer makes progress.
- Thread metadata (#359): best-effort updates wait at most 500 ms, are skipped while the DB is
  known busy, and are applied with the thread's next update or at shutdown.
- Throttle (#351): steps wait about 1 s instead of 10 s.
- Delete (#324): one bounded maintenance per delete, after the hour's validation on a read snapshot.

The ledger stays in the state DB. Splitting it out would separate the locks, but conversation
deletion must remove a conversation's rows and its ledger records together, every ledger write
checks the owner's `threads` row, the 540 MB would have to move out of the live DB, and older
builds sharing the home would keep writing the old location. With the indexes a ledger write holds
the lock for milliseconds, so the split is not needed.

## Load test: `load.py`

`load.py --bin <corbanu> --db <copy> --out <dir> --sessions 10 --turns 3 --tool-calls 4 [--warmup]`
runs 10 concurrent `corbanu exec` sessions (3 turns each, 5 model requests per turn) on one
disposable home holding the DB copy, against `mock_anthropic.py`, with developer accounting on.
`--warmup` runs one exec first so the home's other runtime DBs exist and the hour's validation is
done (ten simultaneous first launches race creating `pfterminal_goals_1`/`memories_1`; that is a
separate, pre-existing bug seen in both builds). Reports are in `evidence/`.

The Mac was shared with other builds (load average 31-45), so absolute times are noisy.

| N=10, warm home | Before (main) | After |
| --- | --- | --- |
| Gap between a tool call's response and the next request, p50 / p99 / max | 20.1 s / 40.9 s / 49.9 s | 0.32 s / 1.56 s / 2.36 s (earlier run: 0.15 / 0.88 / 0.91 s) |
| State DB lock wait p50 / p99 / max | 245 ms / 5.17 s / 5.46 s | 4 ms / 577 ms / 603 ms (earlier run: 0 / 206 / 562 ms) |
| Lock hold p99 / max | 1.12 s / 2.99 s | 117 ms / 1.23 s (commit incl. WAL checkpoint under load) |
| Write-lock holds > 250 ms | 469, all ledger writes | 3 (2 ledger writes, 1 throttle commit) |
| Requests recorded in the ledger | 147 of 150 (5 sent unrecorded) | 150 of 150, none left saved |
| busy/locked/failed lines on stderr | 144 (256 "database is locked") | 7 throttle retries, 1 throttle fallback (earlier run: 1) |
| Thread-metadata writes failed | 30 | 0 |
| Failed execs | 0 | 0 |
| `exec` duration p50 | 114 s | 6.0 s |

Cold hour (no warm-up, validation due): before, p99 gap 30.8 s, lock wait p99 5.24 s, 27 of 150
requests unrecorded; after, p99 gap 0.18 s, lock wait p99 3 ms, the validating `exec` waited 29 s
at exit (`run-final-n10.json`).

`cargo test -p codex-state ten_sessions_on_one_home_wait_briefly_and_lose_no_record` is the CI
version: ten runtimes (as ten processes have) on one home make requests (throttle check,
admission, metadata write, usage) while another connection holds the write lock for 3 s; it
asserts p99 < 1.5 s, max < 2.5 s, no failure, and every record in the ledger.

## Functional check: `glm_tmux_check.sh`

Eight interactive TUI sessions in tmux on one disposable home with a copy of the live DB,
`-m glm-5.3-flash -c model_provider="zai"`, developer accounting on,
`CORBANU_TEST_NO_NATIVE_KEYRING=1`, key from the vault (never printed). Each ran three shell tool
calls and answered. All eight finished (`evidence/glm-8-pane-s1.txt`); no busy, locked or
accounting lines in any pane or in the home's log DB; 30 attempts with 30 observations across the
8 conversations in the ledger, none left pending. (In a first run one of the eight TUIs exited at
start, before its pane could be kept, with nothing in the home's log DB; that run's other seven
finished with no busy or locked lines. The script now keeps exited panes; the rerun above had
none.)
