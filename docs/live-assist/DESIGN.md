# Live meeting assistance

Status: design, nothing built. Written 2026-08-29.

Two panes alongside a running meeting. One says what is being discussed; the
other says what you might do about it. Both are fed by the live transcript,
both go to a model, and neither ever touches the transcript itself.

The hard parts are not the panes. They are cadence, staleness, and earning
attention: a pane that is two minutes behind is worse than no pane, and a
suggestion you learn to ignore has cost you the one glance you had to spare.

---

## What the app already provides

Worth stating, because it decides much of the shape.

- **The live transcript arrives as finals**, per track, with `You` and `Others`
  labels that come from which socket heard them. Self-versus-others is
  therefore free — it is the one piece of speaker attribution this app has
  without a diarizer.
- **The model layer is already provider-agnostic** (`ai/provider.rs`), so Qwen
  on the Spark, Grok, or any OpenAI-compatible endpoint works today. Anthropic
  does not: it is a different request shape, and adding it is a separate piece
  of work rather than a setting.
- **Hindsight is reachable** at `https://hindsight.jtpa.net` —
  `POST /v1/default/banks/{bank_id}/memories/recall` and `/reflect`, no auth
  scheme declared. Verified 2026-08-29.
- **The recogniser can fall behind.** It has, on a loaded appliance, by two
  minutes. Anything built here has to assume the transcript it is reading is
  not the conversation happening in the room.

---

## Two jobs, two rhythms

The instinct is two panes on one timer. That is wrong, because the two answer
different questions and cost different amounts.

### The brief — steady, cheap, incremental

*What is being discussed.* Updated on a timer, roughly every 60–90 seconds.

Sent as **previous brief + segments since**, not the whole transcript. A
meeting's transcript grows without bound and re-summarising it every minute
costs more each time, for a result that changes less each time. The brief is
the accumulator.

The cost of that choice, stated because it is real: an incremental summary can
drift, since each pass sees its own last output rather than the source. A full
re-summarisation on a longer cycle — every ten minutes, say — corrects the
drift without paying for it every minute.

### The suggestions — reactive, expensive, event-driven

*What you might say.* **Not on a timer.** A suggestion is worth making when
something happened: the other side asked a question, or raised something you
have material on. Firing every 90 seconds regardless produces filler, and
filler is how a pane earns being ignored.

The trigger is cheap and local: new `Others` text that looks like a question,
or that shifts topic. A rough heuristic is enough to decide whether to spend a
model call — it does not have to be right, only cheap and roughly right, and
it gates the expensive part.

The expensive part is one pass carrying:

- what **they** just said, verbatim;
- what **you** have already said on that subject, so it does not suggest a
  point you made five minutes ago;
- what **recall** returned from Hindsight for their words;
- what has already been suggested and not used, so the pane does not repeat
  itself.

---

## Never queue

The single most important rule, and the one the appliance has already taught
us.

While a pass is in flight, new transcript marks the state dirty and nothing
else. When it returns, one more pass runs if anything changed. Passes never
stack.

A queue would be catastrophic here rather than merely slow: a model a minute
behind on a sixty-minute meeting ends up answering the first ten minutes for
the whole hour, confidently, while the room has moved on. Dropping intermediate
states is correct — only the current state of the conversation matters.

**Every pane shows how old it is.** Not a spinner: the time of the last
transcript it saw. "As of 14:32" against a clock reading 14:34 is information
the reader needs to decide whether to trust it, and it is exactly what a
spinner hides.

---

## The buttons

The suggestion pass returns options, not prose:

```json
{
  "questions_open": ["Do we have SOC 2?"],
  "options": [
    {"label": "Answer directly", "angle": "..."},
    {"label": "Turn to the audit finding", "angle": "..."},
    {"label": "Defer and follow up", "angle": "..."}
  ]
}
```

Each becomes a button. Pressing one runs a second, focused pass on that angle
alone and returns something you can actually say — which is a different and
better prompt than "suggest a response", because it has been told which of
several directions you have chosen.

Structured output means malformed JSON is a failure mode, not a hypothetical:
a partial response, a model that decides to explain itself first, a reasoning
model that puts its answer somewhere else. The parse must fail to a plain-text
suggestion rather than an empty pane, and never to a fabricated option.

---

## Rules this inherits

**Nothing generated enters the transcript.** The transcript is what was said;
it is what the chain hashes and what a receipt attests. Briefs, suggestions and
recalled memories are shown beside it and stored against the note, never in it.
Screenshots already established this and the reasoning is identical.

**Continuous remote summarisation is a governed action.** `SCOPING.md` puts
"summarize via a remote model endpoint" in the receipt-governed column. Doing it
every ninety seconds for an hour is several dozen such actions, and a receipt
per pass would be a chain nobody can read. **This needs a decision, not a
default** — the plausible answer is one receipt per session, minted when live
assistance is switched on, describing the endpoint and the note. It is not
mine to settle.

---

## Open questions

1. **Which memory, and what is in it?** Hindsight is reachable, but a bank has
   to be named, and the value of every suggestion depends entirely on what that
   bank knows. Recall against a bank holding no sales material returns nothing
   useful, and the pane will look broken when it is merely empty. Note67's own
   past meetings are arguably the better first source, and they are local.

2. **One receipt per session, or none?** Per the governed-action point above.

3. **How much may it interrupt?** The panes are passive by construction. A
   suggestion that pushes itself forward mid-sentence is a different product
   and a much riskier one.

4. **Fable or Anthropic?** Not reachable through the current provider enum, and
   worth knowing whether it is wanted before the interface hardens around an
   OpenAI-compatible shape.
