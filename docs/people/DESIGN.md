# People and participants

Status: design, nothing built. Written 2026-08-21.

Today a transcript's `speaker` column holds one of three things, and none of
them is a person:

- `"You"` / `"Others"` — which track the audio came from. Correct about the
  microphone, and says nothing about who was in the room.
- `"Speaker 1".."Speaker N"` — a diarizer's placeholder, already classified as
  such by `merge::is_generic`.
- A real name, but only where someone typed one, and only inside that one note.

So the questions a meeting archive exists to answer cannot be asked. *What did
Sarah say about the migration? Which meetings was she in? Who has been in every
conversation about this?* The transcripts contain the answers and the app has no
way to reach them.

---

## What the code already decided

Four things are settled by existing behaviour rather than open for design. Each
was verified in the source, not assumed.

**1. The speaker is part of the hashed content.** `note67-canonical`'s
serialization is `<start_ms>\t<end_ms>\t<speaker>\t<text>\n`. Naming a speaker
therefore changes the transcript hash, and `set_segment_speaker` already appends
a chain version for it — deliberately, because *"the chain would otherwise have
a blind spot exactly where the most human-meaningful edits happen."*

The consequence for this design is arithmetic: renaming six speakers one at a
time produces six transcript versions, and six receipts if the note is attested.
That is not a chain we want to read back.

**2. Nothing about a segment survives re-transcription.**
`replace_transcript_segments` deletes every row for the note and re-inserts, so
autoincrement ids change. The text changes too, and so do the boundaries — a
different pass segments the audio differently. There is no stable segment
identity to attach a label to. The only durable anchors are the note, the audio,
and time.

**3. `set_segment_speaker` carries the origin forward** rather than stamping
`Merged`. A hand-typed name is the strongest attribution available, not a
borrowed one. This design must not undo that.

**4. Tags exist and do not sync.** The change feed carries notes,
`note_children` and `transcript_versions`. Anything modelled like tags is
device-local by default, which for people would mean *"which meetings was Sarah
in"* silently means *"on this laptop"*.

---

## The decision that shapes everything else

**Does a segment store a name, or a reference to a person?**

A reference is the instinct — normalise the person, render the name. It is
wrong here, and the chain is what makes it wrong.

The hash is computed over the speaker *string*. If segments held `person_id` and
the name were materialised at hash time, then editing a person's display name
would silently change the hash of every transcript they ever appeared in.
Receipts already minted would no longer verify against their own content. A
typo fix would invalidate the evidence.

**So the transcript keeps the name as text, frozen at the moment it was
applied.** The `people` registry is a separate index that points *at* notes; it
never reaches into hashed content. Renaming a person affects what future
labelling offers, and nothing already recorded.

This costs something honest: correcting "Sara" to "Sarah" in a past meeting is a
transcript edit, appending a version, exactly as it should be. The chain says
the text changed, because it did.

---

## Shape

```
people
  id            TEXT PRIMARY KEY      -- uuid, not autoincrement: has to survive sync
  display_name  TEXT NOT NULL
  is_self       INTEGER NOT NULL      -- exactly one, maps "You" to a real name
  created_at    TEXT NOT NULL
  updated_at    TEXT NOT NULL

note_participants
  note_id       TEXT NOT NULL
  person_id     TEXT NOT NULL
  label         TEXT                  -- the speaker string they were given in
                                      -- this note ("Speaker 2"), for re-applying
  created_at    TEXT NOT NULL
  PRIMARY KEY (note_id, person_id)
```

`label` is what lets a re-diarized transcript be re-labelled without guessing,
and what makes "who was in this meeting" answerable without scanning segments.

Ids are uuids and rows carry `updated_at` so this can join the change feed later
without a migration — see Open questions.

## Flows

**Naming speakers in a note.** The transcript view already lets a speaker be
clicked and named (`0eeab0a`). It becomes: pick from known people or create one,
applied to every segment carrying that label, **in one transaction that appends
exactly one transcript version** however many speakers were assigned. That is
the fix for the arithmetic above, and it wants a new command rather than looping
`set_segment_speaker`.

**Re-transcription.** Auto-retranscribe must not run over a transcript carrying
human labels. This is the same failure as the streaming one already fixed: a
background pass "improving quality" by discarding the most valuable thing in the
note. Manual retranscribe stays available, says plainly that labels will be
lost, and offers to re-apply them by time overlap against `note_participants`.

**Filtering.** Today search is text plus one tag (`selectedTag: string | null`).
People is a second facet, and the two should compose rather than replace each
other. That is the whole of item #6 — the filtering complaint and the people
feature are the same feature, which is why they were asked for together.

---

## Deliberately not in scope

**Voice prints.** The version that recognises Sarah in a meeting she has not
been labelled in needs speaker embeddings, and that is a different product with
a different privacy claim: embeddings of named colleagues are **biometric data**,
which the credential's `data_classes` and the consent wording would both have to
state. It also means storing them, syncing them, and deleting them on request.

Phase 1 deliberately stops short. Named people per note, participants, and
filtering are useful on their own and carry no biometric claim at all. Phase 2
can add recognition when there is a reason to take that on.

---

## Open questions

These change the work materially and are not mine to settle.

1. **Does this sync?** Tags do not, and nobody has minded. People are different:
   the value is cross-meeting, and a device-local answer to "which meetings was
   Sarah in" is a wrong answer rather than a partial one. Syncing means new
   entities in the change feed and a counterpart in `note67-sync`. The tables
   above are shaped for it; the feed work is not small.

2. **Is a participant someone who spoke, or someone who was there?** They are
   not the same list. Someone who attended and said nothing has no segments, and
   is still a participant of the meeting. Supporting that means participants can
   exist without any label, which is easy — but it makes "participants" a fact
   about the meeting rather than a view of the transcript, and that is a
   different thing to keep accurate.

3. **Where do names come from?** Typed by hand is the floor. Teams knows the
   real names and `merge/` already aligns a Teams transcript against a recording
   — which would make imported attendee lists the cheapest large win here, and
   is already noted in the backlog. Whether to lean on that now or later changes
   the order of work.
