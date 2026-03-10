# Light Show Verification

You can verify the syntax of a light show file using the `verify-light-show` command:

```
$ mtrack verify-light-show path/to/show.light
```

This will check the syntax of the light show file and report any errors. You can also validate
the show against your mtrack configuration to ensure all referenced groups and fixtures exist:

```
$ mtrack verify-light-show path/to/show.light --config /path/to/mtrack.yaml
```

This will verify that:
- The light show syntax is valid
- All referenced fixture groups exist in your configuration
- All referenced fixtures exist in your configuration
