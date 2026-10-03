# Light Show Verification

You can verify the syntax of a light show file using the `verify-light-show` command:

```
$ mtrack verify-light-show path/to/show.light
```

This will check the syntax of the light show file and report any errors. You can also validate
the show against your mtrack configuration to ensure all referenced groups exist:

```
$ mtrack verify-light-show path/to/show.light --config /path/to/mtrack.yaml
```

This will verify that:
- The light show syntax is valid
- All referenced fixture groups exist in your configuration (a show targets logical groups, not
  fixtures by name)

**Beat-based shows.** `verify-light-show` reads the file on its own, with no song to inherit a
tempo map from. A show that relies on its song's tempo map for `@measure/beat` cues or
`4measures` durations therefore fails here with "requires a tempo section", although it plays
correctly. To check such a show from the command line, give it a `tempo {}` block ([Where the
tempo comes from](cueing.md#where-the-tempo-comes-from)); the web UI's **Validate** button has no
such limit, since it validates with the song's tempo.

`verify-light-show` checks `.light` shows only. Fixture types and venues (`.light`, `.fixture`
and `.venue`) are parsed when the player loads them, and `mtrack import-gdtf` and
`mtrack import-mvr --write` verify what they write.

## In the web UI

- The **Validate** button on a song's lighting editor checks the show's syntax with the song's
  tempo map, before you save ([Timeline Editor](../interfaces/web-ui.md#timeline-editor)).
- The Lighting **Overview** page answers whether the rig is ready — fixture types, the current
  venue, universes patched in olad, groups that resolve — and lists, for every song with a
  show, the lint findings: cues that would do nothing on this venue, and why. See
  [Lighting in the web UI](web-ui.md) and the [lint codes](configuration.md#lint-warnings).

Over [MCP](../interfaces/mcp.md), `validate_lighting` runs the same lint against the current
venue: focus points the venue does not bind, `per: cell` on fixtures with no cells, groups that
cannot do what an effect asks, and similar mistakes that are legal DSL but do nothing.
