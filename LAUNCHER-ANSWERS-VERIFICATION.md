# Windows actions and inline answers

Implemented 31 fixed Windows settings/tool actions from Mycast with keyword search, typed allowlisted dispatch, category icons and successful-use recency. No user-authored commands or shell execution were introduced.

The launcher recognizes local fend calculations and unit conversions plus complete ISO currency pairs. A prominent answer card sits above at most four ordinary results. Enter copies the selected valid answer using Win32 Unicode clipboard ownership and contracts; clipboard failure leaves the launcher open. Settings contain independent Windows, calculator and currency toggles. Expressions are not saved.

Calculation evaluation uses a fresh fend 1.5.9 context, 150 ms interruption deadline, bounded input/output and one worker with latest-query replacement. Query generations reject stale replies. Same-pair amount edits reuse the pending rate request; other edits, provider disablement and dismissal cancel work.

Frankfurter requests include only the currency pair. Responses are validated and bounded, use an eight-second deadline, and rates persist in a bounded six-hour local cache with dated stale/offline fallback. Currency multiplication uses fend decimal evaluation. Same-currency conversion is entirely local.

## Verification

- Workspace unit tests: 199 passed, one optional live lyrics test ignored.
- Workspace check, formatting and Clippy with warnings denied passed.
- Native catalog probe: 248 entries, including 31 Windows actions; existing native icon and owned-shortcut launch checks passed.
- Calculator/cache tests cover detection, arithmetic, units, decimal conversion, invalid responses/dates/cache data, deadlines and stale fallback.
- Live currency probe compiled the actual service source with a fresh isolated cache under target/fx-live. `100 USD to SGD` returned `127.92 SGD`, reference date 2026-11-08. This was a point-in-time provider check, not a guaranteed rate or live trading quote.
- Debug native answer UI passed arithmetic, same-currency conversion and copy/contraction checks; screenshot inspected. An initial idle assertion sampled the spring before settling; the verifier now waits beyond collapse settlement.

Release build passed all 21 native launcher assertions, including the new answer card, local same-currency conversion, copy/contraction and stopped spring scheduling. Log: target/launcher-ui/c9fb6239f9ad45458a2059ffca77f8fa/verification.txt.

Release point-in-time memory samples, solid material, isolated profile, no enabled plugins (Windows Process.PrivateMemorySize64 / WorkingSet64):

| State | Private bytes | Working set |
| --- | ---: | ---: |
| Idle | 70,987,776 | 74,878,976 |
| Application launcher | 111,333,376 | 115,781,632 |
| Calculator | 120,336,384 | 128,901,120 |
| Same-currency conversion | 122,138,624 | 130,584,576 |

Raw samples: target/launcher-ui/c9fb6239f9ad45458a2059ffca77f8fa/memory.csv. All samples are below 150 MB; these are not peak, live-network, GPU/CPU or every-material/plugin benchmarks. Private bytes and resident working set are different metrics.

 Existing hardware/DPI, clipboard-contention and third-party Settings-page coverage limitations remain. The network remains on-demand with no background rate refresh.

All 145 release UI regression checks passed (hover/timer, layout, lyrics, media): target/polish-probe/2cb9754727304bcaa671b72ca25c9921/results.csv. An earlier debug run under compiler load missed two intermediate lyric-animation samples; the unloaded release rerun passed without changes to lyric code or assertions. All 38 carousel checks passed: target/pages-probe/b3328c7e1af94fa78ddb7e7b51f55cd7/verification.txt.
