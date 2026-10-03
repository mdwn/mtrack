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

`verify-light-show` checks `.light` shows only. Fixture types and venues (`.light`, `.fixture`
and `.venue`) are parsed when the player loads them, and `mtrack import-gdtf` and
`mtrack import-mvr --write` verify what they write. Over [MCP](../interfaces/mcp.md),
`validate_lighting` also lints a show against the current venue: focus points the venue does
not bind, `per: cell` on fixtures with no cells, groups that cannot do what an effect asks, and
similar mistakes that are legal DSL but do nothing.
