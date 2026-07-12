# Doctor jobs from Anthony's podcast

## Source

This note uses the local transcript set generated on July 8, 2026:

`/Users/jaybhagat/Documents/anthony/outputs/podcast-transcripts-20260708`

The manifest lists 18 usable transcripts from the Digital Health Podcast. The
notes below focus on guests who are physicians, dental professionals, or people
who work directly with practices. The line references point to the local text
files. They let a reviewer return to the speaker's full answer instead of using
an invented persona.

## What the doctors need

### They need a short path to first use

Dr. Ashok described moving to virtual care without training. An older public
system took six weeks to activate. His daughter helped him start a simpler tool
in 30 minutes. He kept using it because patients avoided travel and felt calmer
at home.

Source: episode 1, lines 22 to 24.

Product requirement: a clinician should reach a useful synthetic workflow in
one session without installing or learning infrastructure. We should measure
time to first useful result and the number of times a facilitator helps.

### The tool must support the human relationship

Dr. Ashok said AI is a tool and cannot replace the emotional response of a
person. Dr. Augusta Uwah said technology should make physicians more efficient
so they can focus on the interaction. Andreina Sucre said responsibility stays
with the professional because patients trust the person in front of them.

Sources:

- Episode 1, lines 30 to 34.
- Episode 8, lines 26 to 34.
- Episode 20, lines 70 to 72.

Product requirement: generated apps may summarize, organize, and route work.
They must not make an autonomous clinical decision or hide the person who is
responsible. Human review and sign off must remain visible.

### The tool must remove work instead of adding another system

Dr. Uwah said adoption stalls when physicians do not know whether a tool is
safe or when it adds friction to an existing workflow. She also warned that
adding automation to a broken system can make the broken process worse.
Steve Craig described practices with many AI tools that do not exchange data.

Sources:

- Episode 8, lines 38 to 48.
- Episode 21, lines 54 to 58.

Product requirement: each pack must name the current task it replaces. A test
must show that the doctor completes the task with fewer handoffs. A new runtime
service, setting, or dependency needs evidence that the user benefit pays for
the added operating work.

### Safety claims need evidence the practice can understand

Dr. Uwah named governance, BAA coverage, HIPAA obligations, and minimum
necessary data as conditions that help physicians trust a tool. Steve Craig
described practice protocols that protect patients and staff and that vary by
the needs and budget of each group.

Sources:

- Episode 8, line 38.
- Episode 21, lines 20 to 32.

Product requirement: a gate must name the evidence behind its result. A stub
cannot unlock real patient data. Practices must be able to change a protocol
without removing the safety test that protects it.

### Small practices have little spare operating capacity

Andreina Sucre described doing her own content, administration, and planning.
Trying to publish every day caused burnout within a month. Steve Craig described
dental practices as small businesses that need to keep seeing patients and act
fast when a protocol fails.

Sources:

- Episode 20, lines 28 to 36.
- Episode 21, lines 34 to 36.

Product requirement: the managed path needs few choices and no infrastructure
work. The exported path needs a source map, a safe first change, and one command
that proves the app still works.

### The practice needs to keep its own protocol

Steve Craig said each group needs a custom protocol because its operation and
budget differ. Dr. Uwah builds her own apps and agents. Andreina Sucre uses AI
to explain the same clinical topic differently for a professional and a patient.

Sources:

- Episode 21, lines 26 to 28.
- Episode 8, line 16.
- Episode 20, lines 16 to 22.

Product requirement: a pack is a starting point. The doctor must be able to
change language, thresholds, roles, and workflow steps, then export the result
as a repository and a new pack.

## Validation sessions

Use the same task sequence for each doctor:

1. Describe one problem from their practice and choose a pack.
2. Complete the main job with synthetic data.
3. Explain the safety boundary in their own words.
4. Change one practice rule or workflow step.
5. Run the quality check and fix any failure.
6. Export the repository and start it on a clean machine.
7. Reimport the changed pack and give it to another staff member.

Record these measures:

- Time to first useful result.
- Time to first safe change.
- Number of facilitator interventions.
- Task completion without a workaround.
- Correct explanation of the safety boundary.
- Successful export, clean build, and reimport.
- Runtime disk, memory, and startup time.

The current synthetic persona suite checks language variation and regressions.
It does not replace these sessions with the doctors.
