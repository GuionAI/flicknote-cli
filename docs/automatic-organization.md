# Retired client automatic organization (#3634)

The GUI Jev coordinator, OpenRouter Decisions provider, Automatic organization
control, Catch up, cutoff preferences reader, Keychain adapter and probability
routing entrypoints have been removed. Automatic project selection belongs to
the backend; manual project management and note classification remain available.
No client startup migration or credential/preference cleanup runs. Existing
`gui-organization.json` and Keychain items remain untouched and unused.

Project editors now use **description**, not project summary; note summaries
remain independent. Current behavior is documented in the
[normal GUI guide](normal-gui-host.md#projects-and-descriptions-3384-3634).
The [one-time dev migration](project-description-migration.md) preserves all
project assignments and removes only exact old Jev routing blocks. Retiring the
client does not establish online migration, deployment or cloud processing.
Historical #3384/#3398 implementation reports describe the former feature and
must not be treated as current operator instructions.
