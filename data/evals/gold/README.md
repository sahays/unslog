# Gold set — eval reference data

This directory holds the "known-good" outputs that the eval suite grades
against.

**What's committed vs local-only:**

* `adversarial/` — hand-written prompt-injection cases. Committed; safe to
  publish.
* `stories/` and `companies/` — extracted from your live Mongo by `eval
  extract`. **Local-only** (gitignored). Contains real story content with
  real employer names and dollar amounts.

The extracted gold is the contract the suite grades against locally, but
it isn't shared. If we ever need a portable gold set for CI or another
machine, that would be a separate `gold/curated/` tree of sanitized cases,
opt-in committed.

## What's here

```
gold/
├── stories/<story_id>.json    # completed stories + their current StoryVersion body
└── companies/<company_id>.json # companies with a research packet
```

Each JSON is one "accepted" output that the user kept around long enough for
it to count as gold. Practice is intentionally excluded — there's no
per-critique "user accepted this" signal in the app.

## How it gets here

```
cargo run --bin eval -- extract
```

The extractor reads Mongo (`MONGO_URI` / `MONGO_DB` from env) and overwrites
every matching JSON file. Idempotent: re-running picks up new completed
stories and new companies without disturbing existing entries you've edited
out of the gold set.

## What to do with it

**Prune.** "Completed" in the app means "the user clicked Generate at least
once and didn't trash the story" — that's a weaker signal than "this is what
good looks like." After every `extract`, skim the new files and **delete any
entry you wouldn't want the eval suite using as a reference for what a 5/5
output looks like**. Pruning is normal and expected.

**Edit.** If a gold entry is mostly good but has one wrong bullet or a
sentence you'd phrase differently, edit the JSON directly. Plain text,
versioned in git, no migration needed.

**Don't auto-regenerate.** The extractor will happily overwrite local edits.
If you've curated a story to be the canonical reference for a competency,
either move it to `stories/curated/` (which the extractor doesn't touch — and
which `score` and `regression` ignore until you wire it in) or just be aware
that the next `extract` will need a re-prune.

## What runs against it

- `eval score` — runs cheap rubric checks (refusals, length, structure, prompt
  leakage) and the grok-4.3 LLM judge (5 dimensions per target) on every
  gold entry, writes a Markdown report under `data/evals/reports/<ts>/`.
- `eval regression --prompt <name> --baseline <vid> --candidate <vid>` —
  replays each gold entry's input through the baseline vs candidate prompt
  version (supported: `story_summarize` today), diffs outputs, optionally
  asks the judge to score the candidate.
