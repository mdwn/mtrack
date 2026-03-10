# Cueing Features

Light shows support flexible cueing with time-based and measure-based timing, loops, sequences, and offset commands.

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

**Example:**
```light
show "Time-Based Show" {
    @00:00.000
    front_wash: static color: "blue", dimmer: 0%

    @00:05.000
    front_wash: static color: "blue", dimmer: 100%

    @00:10.500
    movers: cycle color: "red", color: "green", speed: 2.0
}
```

## Measure-Based Cues

When a tempo section is defined, cues can use measure/beat notation that automatically adjusts to tempo changes.

**Format: `@measure/beat` or `@measure/beat.subdivision`**
```light
@1/1         # Measure 1, beat 1
@2/3         # Measure 2, beat 3
@4/1.5       # Measure 4, halfway through beat 1
@8/2.75      # Measure 8, three-quarters through beat 2
```

**Example with tempo:**
```light
tempo {
    start: 0.0s
    bpm: 120
    time_signature: 4/4
}

show "Measure-Based Show" {
    @1/1
    front_wash: static color: "red", dimmer: 100%

    @2/1
    back_wash: static color: "blue", dimmer: 100%

    @4/2.5
    movers: strobe frequency: 1beat, duration: 2measures
}
```

## Tempo Sections

Tempo sections define BPM, time signature, and tempo changes throughout the show.

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
- `transition`: Duration of tempo change - number of beats, `Xm` for measures, or `snap` for instant

## Inline Loops

Repeat a block of cues inline without defining a separate sequence.

**Syntax:**
```light
@00:10.000
loop {
    @0.000
    front_wash: static color: "red", dimmer: 100%

    @0.500
    front_wash: static color: "blue", dimmer: 100%

    @1.000
    front_wash: static color: "green", dimmer: 100%
} repeats: 4
```

Timing inside loops is relative to the loop start time. The example above creates 4 cycles of red-blue-green, each cycle taking 1 second.

## Sequences (Subsequences)

Define reusable cue sequences that can be referenced multiple times.

**Defining a sequence:**
```light
sequence "Verse Pattern" {
    @1/1
    front_wash: static color: "blue", dimmer: 80%

    @2/1
    front_wash: static color: "red", dimmer: 100%

    @4/1
    front_wash: static color: "blue", dimmer: 80%
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
- `loop`: Number of times to loop (`once`, `loop` for infinite, or a number)

## Measure Offsets

Shift the measure counter for subsequent cues, useful for complex timing, reusing sequences at different positions, or aligning with composition tools that use repeats.

**Offset command:**
```light
@8/1
offset 4 measures    # Shift measure counter forward by 4 measures
# Next cue at @8/1 will actually be at measure 12

@12/1
reset_measures      # Reset measure counter back to actual playback time
```

**Example use case:**
```light
show "Complex Timing" {
    @1/1
    front_wash: static color: "red", dimmer: 100%

    @4/1
    offset 8 measures    # Shift forward 8 measures
    # Now @4/1 actually plays at measure 12

    @4/1
    back_wash: static color: "blue", dimmer: 100%  # Plays at measure 12

    @8/1
    reset_measures       # Reset counter
    # Now back to actual playback time

    @9/1
    movers: strobe frequency: 4  # Plays at actual measure 9
}
```

## Using Composition Tools as Reference

When composing light shows, you can use tools like Guitar Pro, MuseScore, or other notation software as a reference. These tools often use repeat signs that make measure numbers in the score differ from actual playback position.

**The Problem:**
In Guitar Pro, if you have a 4-measure intro that repeats 3 times, the score might show:
- Measures 1-4: Intro (first time)
- Measures 1-4: Intro (repeat 1)
- Measures 1-4: Intro (repeat 2)
- Measure 5: Verse starts

But in actual playback, measure 5 appears at measure 13 (4 + 4 + 4 + 1). If you write your light show using the score's measure numbers, cues won't align with playback.

**The Solution:**
Use `offset` commands to shift the measure counter to match where sections actually play:

```light
tempo {
    start: 0.0s
    bpm: 120
    time_signature: 4/4
}

show "Song with Repeats" {
    # Intro section (measures 1-4, plays 3 times)
    # First time through
    @1/1
    front_wash: static color: "blue", dimmer: 50%

    @4/1
    front_wash: static color: "blue", dimmer: 100%

    # After first repeat (4 measures later)
    offset 4 measures
    @1/1
    back_wash: static color: "red", dimmer: 50%  # Actually plays at measure 5

    @4/1
    back_wash: static color: "red", dimmer: 100%  # Actually plays at measure 8

    # After second repeat (8 more measures from start, 4 from previous offset)
    offset 4 measures
    @1/1
    movers: strobe frequency: 2  # Actually plays at measure 9

    @4/1
    movers: strobe frequency: 4  # Actually plays at measure 12

    # Verse starts at measure 13 (after 3x4 measure intro)
    offset 4 measures
    @1/1
    reset_measures  # Reset to actual playback time
    # Now we're at measure 13 in actual playback

    @1/1
    all_lights: static color: "green", dimmer: 100%  # Plays at actual measure 13

    @4/1
    all_lights: cycle color: "green", color: "yellow", speed: 2.0  # Plays at measure 16
}
```

**Workflow:**
1. Create your light show using measure numbers from your composition tool (Guitar Pro, etc.)
2. Identify where repeats occur and calculate the cumulative offset
3. Add `offset X measures` commands after each repeat section
4. Use `reset_measures` when you want to return to actual playback time
5. Continue with measure numbers that match actual playback

**Example with Guitar Pro Structure:**
```
Guitar Pro Score Structure:
- Measures 1-4: Intro (repeats 3x)
- Measures 5-12: Verse
- Measures 13-16: Chorus
- Measures 17-20: Verse (repeat)
- Measures 21-24: Chorus (repeat)
- Measure 25: Outro

Actual Playback:
- Measures 1-12: Intro (3x4 measures)
- Measures 13-20: Verse
- Measures 21-24: Chorus
- Measures 25-28: Verse (repeat)
- Measures 29-32: Chorus (repeat)
- Measure 33: Outro
```

```light
show "Guitar Pro Aligned Show" {
    # Intro section (measures 1-4, plays 3 times = 12 measures total)
    @1/1
    front_wash: static color: "blue", dimmer: 30%

    @4/1
    front_wash: static color: "blue", dimmer: 100%

    # After intro repeats, offset by 12 measures (3 repeats × 4 measures)
    offset 12 measures

    # Verse (score shows measures 5-12, actually plays at 13-20)
    @5/1
    reset_measures  # Reset to actual playback (now at measure 13)
    all_lights: static color: "green", dimmer: 80%

    @12/1
    all_lights: cycle color: "green", color: "yellow", speed: 1.5

    # Chorus (score shows measures 13-16, actually plays at 21-24)
    @13/1
    all_lights: static color: "red", dimmer: 100%

    @16/1
    movers: strobe frequency: 8, duration: 1measure

    # Verse repeat (score shows measures 17-20, actually plays at 25-28)
    @17/1
    offset 4 measures  # Chorus was 4 measures, so offset by 4
    reset_measures
    all_lights: static color: "green", dimmer: 80%

    # Chorus repeat (score shows measures 21-24, actually plays at 29-32)
    @21/1
    offset 4 measures
    reset_measures
    all_lights: static color: "red", dimmer: 100%

    # Outro (score shows measure 25, actually plays at measure 33)
    @25/1
    offset 4 measures
    reset_measures
    all_lights: dimmer start_level: 100%, end_level: 0%, duration: 4s
}
```

This approach lets you write light shows using the same measure numbers as your composition tool, making it easier to sync lighting with your musical arrangement.

## Stopping Sequences

Stop a running sequence at a specific cue time.

**Syntax:**
```light
@00:30.000
stop sequence "Verse Pattern"
```

This stops the named sequence if it's currently playing.
