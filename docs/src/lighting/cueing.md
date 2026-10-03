# Cueing Features

Light shows support time-based and measure-based cue timing, inline loops, reusable sequences,
measure offsets, and commands that act on whole layers.

## Where the tempo comes from

A show needs a tempo map to use anything musical: `@measure/beat` cues, `4measures` durations,
`1beat` frequencies. The map comes from one of two places:

- **The song's tempo map** (the default). A `.light` file with no `tempo {}` block is parsed with
  the tempo of the song it belongs to: the song's `tempo:` block in `song.yaml` if it has one,
  otherwise the beat grid mtrack derives from the song's click track. See
  [Song Configuration](../configuration/song-config.md).
- **A `tempo {}` block in the show** overrides the song's map. Use it when the show needs a
  tempo the song does not carry, or when the file is verified on its own: `mtrack
  verify-light-show` runs without a song, so a beat-based show needs the block to pass there
  ([Verification](verification.md)).

A show that uses musical timing with neither fails to parse with "requires a tempo section".

## Time-Based Cues

Cues can be specified using absolute time in two formats:

**Format 1: Minutes:Seconds.Milliseconds**
```light
@00:05.000    # 5 seconds
@01:23.456    # 1 minute, 23.456 seconds
@02:00.000    # 2 minutes
```

**Format 2: Seconds.Milliseconds**
```light
@5.000        # 5 seconds
@83.456       # 83.456 seconds
@120.000      # 120 seconds (2 minutes)
```

The fractional part is required: `@5` on its own is a syntax error, because without a `/` it is
not a measure and without a `.` it is not a time. Write `@5.000`.

**Example:**
```light
show "Time-Based Show" {
    @00:00.000
    front_wash: static color: "blue", dimmer: 0%, duration: 5s

    @00:05.000
    front_wash: static color: "blue", dimmer: 100%, duration: 5s

    @00:10.500
    movers: cycle color: "red", color: "green", speed: 2.0, duration: 10s
}
```

## Measure-Based Cues

With a tempo map (the song's, or a `tempo {}` block), cues can use measure/beat notation that
follows tempo changes.

**Format: `@measure/beat` or `@measure/beat.subdivision`**
```light
@1/1         # Measure 1, beat 1
@2/3         # Measure 2, beat 3
@4/1.5       # Measure 4, halfway through beat 1
@8/2.75      # Measure 8, three-quarters through beat 2
```

Measures and beats are 1-based: `@1/1` is the first downbeat.

**Example with tempo:**
```light
tempo {
    start: 0.0s
    bpm: 120
    time_signature: 4/4
}

show "Measure-Based Show" {
    @1/1
    front_wash: static color: "red", dimmer: 100%, duration: 4measures

    @2/1
    back_wash: static color: "blue", dimmer: 100%, duration: 4measures

    @4/2.5
    movers: strobe frequency: 1beat, duration: 2measures
}
```

## Tempo Sections

A `tempo {}` block defines BPM, time signature, and tempo changes. It goes at file scope
(applying to every show and sequence in the file) or as the first item inside a `show { }`
body. `start` is where measure 1 beat 1 falls in the song: a song with a lead-in before the
click starts needs it set to that lead-in, or every cue lands early by exactly that much.

**Basic tempo:**
```light
tempo {
    start: 0.0s
    bpm: 120
    time_signature: 4/4
}
```

**Tempo with changes:**
```light
tempo {
    start: 0.0s
    bpm: 120
    time_signature: 4/4
    changes: [
        @8/1 { bpm: 140 },                    # Instant change at measure 8
        @16/1 { bpm: 160, transition: 4 },    # Gradual change over 4 beats
        @24/1 { bpm: 180, transition: 2m },   # Gradual change over 2 measures
        @32/1 { time_signature: 3/4 },        # Time signature change
        @40/1 { bpm: 100, transition: snap }  # Instant snap back
    ]
}
```

**Tempo change parameters:**
- `bpm`: New BPM value
- `time_signature`: New time signature (e.g., `3/4`, `6/8`)
- `transition`: Duration of tempo change - a bare number of beats, `Xm` for measures, or `snap`
  for instant

## Inline Loops

Repeat a block of cues inline without defining a separate sequence.

**Syntax:**
```light
@00:10.000
loop {
    @0.000
    front_wash: static color: "red", dimmer: 100%, duration: 500ms

    @0.500
    front_wash: static color: "blue", dimmer: 100%, duration: 500ms

    @1.000
    front_wash: static color: "green", dimmer: 100%, duration: 500ms
} repeats: 4
```

Timing inside loops is relative to the loop start time. The example above creates 4 cycles of
red-blue-green, each cycle taking 1 second.

## Sequences (Subsequences)

Define reusable cue sequences that can be referenced multiple times.

**Defining a sequence:**
```light
sequence "Verse Pattern" {
    @1/1
    front_wash: static color: "blue", dimmer: 80%, duration: 4measures

    @2/1
    front_wash: static color: "red", dimmer: 100%, duration: 2measures

    @4/1
    front_wash: static color: "blue", dimmer: 80%, duration: 4measures
}
```

**Referencing a sequence:**
```light
show "Song" {
    @1/1
    sequence "Verse Pattern"

    @17/1
    sequence "Verse Pattern"  # Reuse the same pattern

    @33/1
    sequence "Verse Pattern", loop: 2  # Loop the sequence twice
}
```

**Sequence parameters:**
- `loop`: How many times to play the sequence: a number, `once` (the default), or `loop`, which
  repeats it 10,000 times — in practice until the song ends or a `stop sequence` command stops
  it. `pingpong` and `random` are accepted by the grammar but rejected at parse time as not
  implemented.

## Measure Offsets

`offset N measures` shifts the measure numbering for the cues that follow it, so a cue written
as `@M/B` plays at measure `M + N`. `reset_measures` puts the numbering back so `@M/B` plays at
measure `M` again. Both need a tempo map.

Three rules decide what a command affects:

- A command sits inside a cue, after the cue's `@` line, and takes effect from the **next** `@`
  onward. It never moves the cue it is written in.
- Offsets **accumulate**: `offset 4 measures` followed later by another `offset 4 measures` is a
  shift of 8.
- `reset_measures` and `offset` in the same cue read in that order: `reset_measures` then
  `offset 4 measures` leaves a shift of exactly 4, whatever came before.

The shift is worked out in seconds at the tempo in force where the command is issued, and it
moves the tempo map's `changes` along with the cues, so a change the block places at `@8/1`
happens at the shifted measure too.

**Example:**
```light
tempo {
    start: 0.0s
    bpm: 120
    time_signature: 4/4
}

show "Offset Rules" {
    @1/1
    front_wash: static color: "red", dimmer: 100%, duration: 2measures

    @4/1
    front_wash: static color: "red", dimmer: 50%, duration: 1measure
    offset 8 measures      # from the next cue on, @M/B means measure M + 8

    @4/1
    back_wash: static color: "blue", dimmer: 100%, duration: 2measures   # plays at measure 12

    @8/1
    back_wash: static color: "blue", dimmer: 50%, duration: 1measure     # plays at measure 16
    reset_measures         # from the next cue on, @M/B means measure M again

    @17/1
    movers: strobe frequency: 4, duration: 1measure                      # plays at measure 17
}
```

## Using Composition Tools as Reference

Notation software such as Guitar Pro or MuseScore numbers measures as the score prints them,
and a repeat sign plays the same numbered measures again. Playback measure numbers run on
without repeating, so a score that reads "intro, measures 1–4, play three times; verse from
measure 5" has its verse at playback measure 13, not 5.

Offsets let the show use the score's numbers. Each pass through a repeat adds the repeat's
length to the shift, and the shift stays in force for everything after the repeats:

```light
tempo {
    start: 0.0s
    bpm: 120
    time_signature: 4/4
}

show "Song with Repeats" {
    # Intro, first time: score measures 1-4 are playback measures 1-4
    @1/1
    front_wash: static color: "blue", dimmer: 50%, duration: 3measures

    @4/1
    front_wash: static color: "blue", dimmer: 100%, duration: 1measure
    offset 4 measures      # second time through: score measure 1 is playback measure 5

    # Intro, second time
    @1/1
    back_wash: static color: "red", dimmer: 50%, duration: 3measures     # playback measure 5

    @4/1
    back_wash: static color: "red", dimmer: 100%, duration: 1measure     # playback measure 8
    offset 4 measures      # third time through: a shift of 8 from here on

    # Intro, third time
    @1/1
    movers: strobe frequency: 2, duration: 3measures                     # playback measure 9

    @4/1
    movers: strobe frequency: 4, duration: 1measure                      # playback measure 12

    # Verse: the score says measure 5, and the shift of 8 is still in force
    @5/1
    all_lights: static color: "green", dimmer: 100%, duration: 4measures # playback measure 13

    @9/1
    all_lights: cycle color: "green", color: "yellow", speed: 2.0, duration: 8measures  # playback measure 17
}
```

**Workflow:**
1. Write cues with the measure numbers your composition tool shows
2. At the end of each pass through a repeated section, add `offset X measures`, where `X` is the
   length of the repeated section
3. Leave the accumulated offset in force for the rest of the score, since every later score
   measure is that much later in playback
4. Use `reset_measures` only where you want to go back to playback numbering

## Commands

A cue can carry commands as well as effects. Each goes on its own line under the cue's `@`
time, and a cue may hold several.

| Command | What it does |
|---|---|
| `clear()` | Stops every effect on every layer at once, and resets every layer's master and freeze. It also releases movers' pose memory, so the next `move` starts from its target rather than from where the heads were. |
| `clear(layer: midground)` | Stops every effect on that one layer, and resets that layer's master and freeze. |
| `freeze(layer: background)` | Holds every effect on the layer where it is: they stop advancing in time and keep their current output. |
| `unfreeze(layer: background)` | Resumes the layer's effects from where they were frozen. |
| `master(layer: foreground, intensity: 50%)` | Scales the layer's output. `speed: 0.5` scales how fast its effects run instead; give one or both. Values are a percentage or a `0.0`–`1.0` number. |
| `stop sequence "Verse Pattern"` | Stops the named sequence if it is playing. |
| `offset 4 measures` | Shifts the measure numbering of the cues that follow ([Measure Offsets](#measure-offsets)). |
| `reset_measures` | Removes the shift for the cues that follow. |

`layer:` is required on `freeze`, `unfreeze` and `master`; only `clear` may omit it, and then it
means every layer. Layers are `background`, `midground` and `foreground`.

`clear` is an immediate cut, not a fade. There is no command that fades a layer out: every
effect has a finite duration, so put the fade on the effect itself with `down_time` and let it
end when it should.

Masters and freezes last for the song that set them. They are reset when playback stops or
another song loads, and by `clear` as above, so a show stopped between a `master` and the cue
that was going to undo it cannot leave the next song dimmed.

**Example:**
```light
show "Commands" {
    @00:00.000
    front_wash: static color: "blue", dimmer: 100%, duration: 60s, layer: background
    movers: cycle color: "red", color: "white", speed: 1.0, duration: 60s, layer: midground

    @00:20.000
    master(layer: midground, intensity: 50%)

    @00:30.000
    freeze(layer: midground)

    @00:35.000
    unfreeze(layer: midground)

    @00:40.000
    clear(layer: midground)

    @00:50.000
    clear()
}
```
